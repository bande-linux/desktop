use std::fs::{self, File};
use std::io::{Read, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use bande_https::ureq;
use serde::Deserialize;
use sha2::{Digest, Sha256, Sha512};

use crate::{Error, Result};

const CHUNK: usize = 1024 * 1024;
const CHECKSUMS_LIMIT: u64 = 1024 * 1024;
const RELEASES_LIMIT: u64 = 16 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Sha256,
    Sha512,
}

#[derive(Clone, Copy, Debug)]
enum Signed {
    Https,
    Detached {
        url: &'static str,
        keys: &'static [u8],
    },
    Inline {
        keys: &'static [u8],
    },
}

#[derive(Clone, Copy, Debug)]
enum Location {
    Fixed {
        url: &'static str,
        checksums: &'static str,
    },
    NewestFedora,
}

#[derive(Clone, Copy, Debug)]
pub struct CloudImage {
    pub os: &'static str,
    location: Location,
    pub algorithm: Algorithm,
    signed: Signed,
}

#[derive(Debug, PartialEq, Eq)]
struct Located {
    url: String,
    checksums: String,
}

#[derive(Deserialize)]
struct FedoraRelease {
    version: String,
    arch: String,
    variant: String,
    link: String,
}

const FEDORA_RELEASES: &str = "https://fedoraproject.org/releases.json";
const FEDORA_DOWNLOADS: &str = "https://download.fedoraproject.org/pub/fedora/linux/releases/";
const FEDORA_IMAGE: &str = "Fedora-Cloud-Base-Generic-";
const NEWEST_SIGNED_FEDORA: u32 = 46;

const GPGV: &str = "gpgv";

#[cfg(target_arch = "x86_64")]
macro_rules! arch {
    (debian) => {
        "amd64"
    };
    (kernel) => {
        "x86_64"
    };
}

#[cfg(target_arch = "aarch64")]
macro_rules! arch {
    (debian) => {
        "arm64"
    };
    (kernel) => {
        "aarch64"
    };
}

const CATALOG: &[CloudImage] = &[
    CloudImage {
        os: "Debian 13 (trixie)",
        location: Location::Fixed {
            url: concat!(
                "https://cloud.debian.org/images/cloud/trixie/latest/debian-13-generic-",
                arch!(debian),
                ".qcow2"
            ),
            checksums: "https://cloud.debian.org/images/cloud/trixie/latest/SHA512SUMS",
        },
        algorithm: Algorithm::Sha512,
        signed: Signed::Https,
    },
    CloudImage {
        os: "Ubuntu 26.04 LTS",
        location: Location::Fixed {
            url: concat!(
                "https://cloud-images.ubuntu.com/resolute/current/resolute-server-cloudimg-",
                arch!(debian),
                ".img"
            ),
            checksums: "https://cloud-images.ubuntu.com/resolute/current/SHA256SUMS",
        },
        algorithm: Algorithm::Sha256,
        signed: Signed::Detached {
            url: "https://cloud-images.ubuntu.com/resolute/current/SHA256SUMS.gpg",
            keys: include_bytes!("../data/ubuntu.gpg"),
        },
    },
    CloudImage {
        os: "Fedora Cloud",
        location: Location::NewestFedora,
        algorithm: Algorithm::Sha256,
        signed: Signed::Inline {
            keys: include_bytes!("../data/fedora.gpg"),
        },
    },
];

pub fn catalog() -> &'static [CloudImage] {
    CATALOG
}

pub struct ImageStore {
    dir: PathBuf,
    agent: ureq::Agent,
}

enum Hasher {
    Sha256(Sha256),
    Sha512(Sha512),
}

impl Algorithm {
    fn hex_len(self) -> usize {
        match self {
            Self::Sha256 => 64,
            Self::Sha512 => 128,
        }
    }

    fn hasher(self) -> Hasher {
        match self {
            Self::Sha256 => Hasher::Sha256(Sha256::new()),
            Self::Sha512 => Hasher::Sha512(Sha512::new()),
        }
    }
}

impl Hasher {
    fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Sha256(hasher) => hasher.update(bytes),
            Self::Sha512(hasher) => hasher.update(bytes),
        }
    }

    fn hex(self) -> String {
        let digest = match self {
            Self::Sha256(hasher) => hasher.finalize().to_vec(),
            Self::Sha512(hasher) => hasher.finalize().to_vec(),
        };
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

fn file_name(url: &str) -> &str {
    url.rsplit('/').next().unwrap_or(url)
}

fn newest_fedora(releases: &[FedoraRelease]) -> Option<Located> {
    let suffix = concat!(".", arch!(kernel), ".qcow2");
    releases
        .iter()
        .filter(|release| release.variant == "Cloud" && release.arch == arch!(kernel))
        .filter_map(|release| {
            let version: u32 = release.version.parse().ok()?;
            let name = file_name(&release.link);
            let compose = name.strip_prefix(FEDORA_IMAGE)?.strip_suffix(suffix)?;
            let folder = release.link.strip_suffix(name)?;
            (version <= NEWEST_SIGNED_FEDORA && folder.starts_with(FEDORA_DOWNLOADS)).then(|| {
                let checksums =
                    format!("{folder}Fedora-Cloud-{compose}-{}-CHECKSUM", arch!(kernel));
                let url = release.link.clone();
                (version, Located { url, checksums })
            })
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, located)| located)
}

impl CloudImage {
    fn listed_checksum(&self, checksums: &str, file: &str) -> Option<String> {
        let hex = checksums.lines().find_map(|line| {
            let line = line.trim();
            match line.split_once(") = ") {
                Some((head, hex)) => (head.split_once(" (")?.1 == file).then_some(hex),
                None => {
                    let (hex, name) = line.split_once(char::is_whitespace)?;
                    (name.trim_start().trim_start_matches('*') == file).then_some(hex)
                }
            }
        })?;
        (hex.len() == self.algorithm.hex_len() && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| hex.to_ascii_lowercase())
    }
}

impl ImageStore {
    pub fn open() -> Result<Self> {
        let data = bande_xdg::data_home().ok_or(Error::Missing("XDG_DATA_HOME".into()))?;
        Ok(Self::at(data.join("velox/images")))
    }

    pub fn at(dir: PathBuf) -> Self {
        let agent = bande_https::config("velox")
            .https_only(true)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_recv_response(Some(CONNECT_TIMEOUT))
            .build()
            .into();
        Self { dir, agent }
    }

    fn locate(&self, image: &CloudImage) -> Result<Located> {
        let (url, checksums) = match image.location {
            Location::Fixed { url, checksums } => (url, checksums),
            Location::NewestFedora => {
                let releases: Vec<FedoraRelease> = serde_json::from_str(
                    &self
                        .agent
                        .get(FEDORA_RELEASES)
                        .call()?
                        .body_mut()
                        .with_config()
                        .limit(RELEASES_LIMIT)
                        .read_to_string()?,
                )?;
                return newest_fedora(&releases).ok_or_else(|| Error::NoRelease(image.os.into()));
            }
        };
        Ok(Located {
            url: url.into(),
            checksums: checksums.into(),
        })
    }

    pub fn fetch(
        &self,
        image: &CloudImage,
        progress: &mut dyn FnMut(u64, Option<u64>) -> ControlFlow<()>,
    ) -> Result<PathBuf> {
        let located = self.locate(image)?;
        let file = file_name(&located.url);
        let checksums = self
            .agent
            .get(&located.checksums)
            .call()?
            .body_mut()
            .with_config()
            .limit(CHECKSUMS_LIMIT)
            .read_to_string()?;
        let checksums = self.verified(image, checksums)?;
        let expected = image
            .listed_checksum(&checksums, file)
            .ok_or_else(|| Error::NotListed(file.into()))?;
        let path = self.dir.join(format!("{}-{file}", &expected[..16]));
        if path.is_file() {
            return Ok(path);
        }
        fs::create_dir_all(&self.dir)?;
        let staging = path.with_extension("partial");
        let downloaded = self.download(image, &located.url, &expected, &staging, progress);
        match downloaded {
            Ok(file) => persist(&file, &staging, &path).map(|()| path),
            Err(error) => {
                let _ = fs::remove_file(&staging);
                Err(error)
            }
        }
    }

    fn verified(&self, image: &CloudImage, checksums: String) -> Result<String> {
        let keys = match image.signed {
            Signed::Https => return Ok(checksums),
            Signed::Detached { keys, .. } | Signed::Inline { keys } => keys,
        };
        fs::create_dir_all(&self.dir)?;
        let folder = self.dir.join(format!(".verify-{}", std::process::id()));
        fs::create_dir_all(&folder)?;
        let checked = self.run_gpgv(image, &checksums, keys, &folder);
        let _ = fs::remove_dir_all(&folder);
        checked
    }

    fn run_gpgv(
        &self,
        image: &CloudImage,
        checksums: &str,
        keys: &[u8],
        folder: &Path,
    ) -> Result<String> {
        let keyring = folder.join("keys.gpg");
        let signed = folder.join("sums");
        fs::write(&keyring, keys)?;
        fs::write(&signed, checksums)?;
        let mut command = Command::new(GPGV);
        command.arg("--keyring").arg(&keyring);
        if let Signed::Inline { .. } = image.signed {
            command.args(["--output", "-"]);
        }
        if let Signed::Detached { url, .. } = image.signed {
            let signature = folder.join("sums.sig");
            let mut bytes = Vec::new();
            self.agent
                .get(url)
                .call()?
                .into_body()
                .into_reader()
                .take(CHECKSUMS_LIMIT)
                .read_to_end(&mut bytes)?;
            fs::write(&signature, bytes)?;
            command.arg(signature);
        }
        let output = command.arg(&signed).stdin(Stdio::null()).output()?;
        if !output.status.success() {
            return Err(Error::Signature(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        Ok(match image.signed {
            Signed::Inline { .. } => String::from_utf8_lossy(&output.stdout).into_owned(),
            _ => checksums.to_owned(),
        })
    }

    fn download(
        &self,
        image: &CloudImage,
        url: &str,
        expected: &str,
        staging: &Path,
        progress: &mut dyn FnMut(u64, Option<u64>) -> ControlFlow<()>,
    ) -> Result<File> {
        let response = self.agent.get(url).call()?;
        let total = response.body().content_length();
        let mut body = response.into_body().into_reader();
        let mut file = File::create(staging)?;
        let mut hasher = image.algorithm.hasher();
        let mut buffer = vec![0; CHUNK];
        let mut done = 0;
        loop {
            let read = body.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            file.write_all(&buffer[..read])?;
            done += read as u64;
            if progress(done, total).is_break() {
                return Err(Error::Cancelled);
            }
        }
        let actual = hasher.hex();
        if actual != expected {
            return Err(Error::Checksum {
                expected: expected.into(),
                actual,
            });
        }
        Ok(file)
    }
}

fn persist(file: &File, staging: &Path, path: &Path) -> Result<()> {
    file.sync_all()?;
    fs::rename(staging, path)?;
    if let Some(folder) = path.parent() {
        bande_xdg::sync_folder(folder)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fedora_is_the_newest_release_its_keys_sign_from_fedoras_mirrors() {
        let arch = arch!(kernel);
        let release = |version: &str, link: String| FedoraRelease {
            version: version.into(),
            arch: arch.into(),
            variant: "Cloud".into(),
            link,
        };
        let images = |version: u32| format!("{FEDORA_DOWNLOADS}{version}/Cloud/{arch}/images/");
        let generic = |version: u32, compose: &str| {
            format!(
                "{}Fedora-Cloud-Base-Generic-{version}-{compose}.{arch}.qcow2",
                images(version)
            )
        };
        let releases = [
            release("44", generic(44, "1.7")),
            release("45", generic(45, "1.4")),
            release(
                "45",
                format!(
                    "{}Fedora-Cloud-Base-UEFI-UKI-45-1.4.{arch}.qcow2",
                    images(45)
                ),
            ),
            release("46 Beta", generic(46, "0.9")),
            release(
                "46",
                format!("https://example.org/Fedora-Cloud-Base-Generic-46-1.2.{arch}.qcow2"),
            ),
            release(
                &(NEWEST_SIGNED_FEDORA + 1).to_string(),
                generic(NEWEST_SIGNED_FEDORA + 1, "1.1"),
            ),
        ];
        assert_eq!(
            newest_fedora(&releases),
            Some(Located {
                url: generic(45, "1.4"),
                checksums: format!("{}Fedora-Cloud-45-1.4-{arch}-CHECKSUM", images(45)),
            })
        );
    }
}
