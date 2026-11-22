use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sequoia_gpg_agent::KeyPair;
use sequoia_gpg_agent::gnupg::Context;
use sequoia_openpgp::cert::{Cert, CertParser};
use sequoia_openpgp::packet::Key;
use sequoia_openpgp::packet::key::{PublicParts, UnspecifiedRole};
use sequoia_openpgp::parse::Parse;
use sequoia_openpgp::{Fingerprint, KeyHandle};

use crate::error::{Error, Result};

const GPG: &str = "gpg";
const GPGSM: &str = "gpgsm";
const GPGCONF: &str = "gpgconf";
const MAX_OUTPUT: usize = 32 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnKey {
    pub fingerprint: String,
    pub user_ids: Vec<String>,
    pub expires: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnCertificate {
    pub fingerprint: String,
    pub subject: String,
    pub emails: Vec<String>,
    pub expires: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GnuPg {
    home: Option<PathBuf>,
}

fn crypto_error(text: impl std::fmt::Display) -> Error {
    Error::Crypto(text.to_string())
}

fn field(line: &str, index: usize) -> &str {
    line.split(':').nth(index).unwrap_or_default()
}

fn unescaped(text: &str) -> String {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        let escaped = (byte == b'\\' && tail.first() == Some(&b'x'))
            .then(|| tail.get(1..3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok());
        match escaped {
            Some(value) => {
                bytes.push(value);
                rest = &tail[3..];
            }
            None => {
                bytes.push(byte);
                rest = tail;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

pub fn email_of(user_id: &str) -> Option<String> {
    let inner = match (user_id.rfind('<'), user_id.rfind('>')) {
        (Some(start), Some(end)) if start < end => &user_id[start + 1..end],
        _ => user_id,
    };
    let inner = inner.trim();
    inner
        .contains('@')
        .then(|| inner.to_lowercase())
        .filter(|email| !email.contains(char::is_whitespace))
}

fn expiry(text: &str) -> Option<i64> {
    text.parse().ok().filter(|seconds| *seconds > 0)
}

fn smime_expiry(text: &str) -> Option<i64> {
    expiry(text).or_else(|| {
        let stamp = jiff::civil::DateTime::strptime("%Y%m%dT%H%M%S", text).ok()?;
        Some(
            stamp
                .to_zoned(jiff::tz::TimeZone::UTC)
                .ok()?
                .timestamp()
                .as_second(),
        )
    })
}

impl GnuPg {
    pub fn at(home: &Path) -> Self {
        Self {
            home: Some(home.to_owned()),
        }
    }

    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        if let Some(home) = &self.home {
            command.arg("--homedir").arg(home);
        }
        if program != GPGCONF {
            command.args(["--batch", "--no-tty", "--status-fd", "2"]);
        }
        if program == GPGSM {
            command.arg("--disable-dirmngr");
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn run(&self, mut command: Command, input: Option<&[u8]>) -> Result<Vec<u8>> {
        if input.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = command.spawn().map_err(|error| {
            crypto_error(bande_l10n::tr!(
                "GnuPG can’t be started: {error}",
                error = error.to_string()
            ))
        })?;
        if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
            let data = input.to_vec();
            std::thread::spawn(move || {
                if let Err(error) = stdin.write_all(&data) {
                    tracing::warn!("cannot hand data to GnuPG: {error}");
                }
            });
        }
        let output = child.wait_with_output()?;
        if output.stdout.len() > MAX_OUTPUT {
            return Err(crypto_error("GnuPG answered with too much data"));
        }
        if !output.status.success() {
            let errors = String::from_utf8_lossy(&output.stderr);
            let reason = errors
                .lines()
                .rfind(|line| !line.starts_with("[GNUPG:]"))
                .unwrap_or("GnuPG failed")
                .trim_start_matches("gpg: ")
                .trim_start_matches("gpgsm: ")
                .to_owned();
            return Err(crypto_error(reason));
        }
        Ok(output.stdout)
    }

    fn listing(&self, program: &str, arguments: &[&str]) -> Result<String> {
        let mut command = self.command(program);
        command.arg("--with-colons").args(arguments);
        Ok(String::from_utf8_lossy(&self.run(command, None)?).into_owned())
    }

    pub fn agent(&self) -> Result<Context> {
        let mut launch = self.command(GPGCONF);
        launch.args(["--launch", "gpg-agent"]);
        self.run(launch, None)?;
        match &self.home {
            Some(home) => Context::with_homedir(home),
            None => Context::new(),
        }
        .map_err(crypto_error)
    }

    pub fn keypair(&self, key: &Key<PublicParts, UnspecifiedRole>) -> Result<KeyPair> {
        KeyPair::new_for_gnupg_context(&self.agent()?, key).map_err(crypto_error)
    }

    fn parse_certs(data: &[u8]) -> Vec<Cert> {
        CertParser::from_bytes(data)
            .map(|parser| {
                parser
                    .filter_map(|cert| {
                        cert.inspect_err(|error| tracing::warn!("skipping a certificate: {error}"))
                            .ok()
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn certs(&self, handles: &[KeyHandle]) -> Vec<Cert> {
        if handles.is_empty() {
            return Vec::new();
        }
        let mut command = self.command(GPG);
        command.arg("--export");
        for handle in handles {
            command.arg(handle.to_hex());
        }
        match self.run(command, None) {
            Ok(data) => Self::parse_certs(&data),
            Err(error) => {
                tracing::warn!("cannot look up OpenPGP keys: {error}");
                Vec::new()
            }
        }
    }

    pub fn certs_for(&self, email: &str) -> Vec<Cert> {
        let mut command = self.command(GPG);
        command.arg("--export").arg(format!("<{email}>"));
        match self.run(command, None) {
            Ok(data) => Self::parse_certs(&data),
            Err(error) => {
                tracing::warn!("cannot look up the OpenPGP key of {email}: {error}");
                Vec::new()
            }
        }
    }

    pub fn certified(&self, fingerprint: &Fingerprint) -> Vec<String> {
        let listing = match self.listing(GPG, &["--list-keys", &fingerprint.to_hex()]) {
            Ok(listing) => listing,
            Err(error) => {
                tracing::warn!("cannot ask GnuPG about {fingerprint}: {error}");
                return Vec::new();
            }
        };
        listing
            .lines()
            .filter(|line| field(line, 0) == "uid" && matches!(field(line, 1), "f" | "u"))
            .filter_map(|line| email_of(&unescaped(field(line, 9))))
            .collect()
    }

    pub fn secret_subkeys(&self) -> Vec<String> {
        let listing = self
            .listing(GPG, &["--list-secret-keys"])
            .inspect_err(|error| tracing::warn!("cannot list secret keys: {error}"))
            .unwrap_or_default();
        let mut available = false;
        let mut fingerprints = Vec::new();
        for line in listing.lines() {
            match field(line, 0) {
                "sec" | "ssb" => available = field(line, 14) != "#",
                "fpr" if available => fingerprints.push(field(line, 9).to_owned()),
                _ => {}
            }
        }
        fingerprints
    }

    pub fn own_keys(&self) -> Result<Vec<OwnKey>> {
        let listing = self.listing(GPG, &["--list-secret-keys"])?;
        let mut keys: Vec<OwnKey> = Vec::new();
        let mut primary = false;
        for line in listing.lines() {
            match field(line, 0) {
                "sec" => {
                    primary = true;
                    let revoked = matches!(field(line, 1), "r" | "e" | "d");
                    if !revoked {
                        keys.push(OwnKey {
                            fingerprint: String::new(),
                            user_ids: Vec::new(),
                            expires: expiry(field(line, 6)),
                        });
                    }
                }
                "ssb" => primary = false,
                "fpr" if primary => {
                    if let Some(key) = keys.last_mut().filter(|key| key.fingerprint.is_empty()) {
                        field(line, 9).clone_into(&mut key.fingerprint);
                    }
                }
                "uid" if !matches!(field(line, 1), "r" | "e") => {
                    if let Some(key) = keys.last_mut() {
                        key.user_ids.push(unescaped(field(line, 9)));
                    }
                }
                _ => {}
            }
        }
        keys.retain(|key| !key.fingerprint.is_empty());
        Ok(keys)
    }

    pub fn import(&self, data: &[u8]) -> Result<()> {
        let mut command = self.command(GPG);
        command.arg("--import");
        self.run(command, Some(data)).map(drop)
    }

    pub fn export(&self, fingerprint: &str) -> Result<Vec<u8>> {
        let mut command = self.command(GPG);
        command.args(["--armor", "--export", fingerprint]);
        let exported = self.run(command, None)?;
        match exported.is_empty() {
            true => Err(crypto_error(bande_l10n::tr!(
                "The key is no longer in GnuPG."
            ))),
            false => Ok(exported),
        }
    }

    pub fn generate(&self, user_id: &str) -> Result<()> {
        let mut command = self.command(GPG);
        command.args([
            "--quick-generate-key",
            user_id,
            "future-default",
            "default",
            "3y",
        ]);
        self.run(command, None).map(drop)
    }

    pub fn own_certificates(&self) -> Result<Vec<OwnCertificate>> {
        let listing = self.listing(GPGSM, &["--list-secret-keys"])?;
        let mut certificates: Vec<OwnCertificate> = Vec::new();
        for line in listing.lines() {
            match field(line, 0) {
                "crs" => certificates.push(OwnCertificate {
                    fingerprint: String::new(),
                    subject: String::new(),
                    emails: Vec::new(),
                    expires: smime_expiry(field(line, 6)),
                }),
                "fpr" => {
                    if let Some(certificate) = certificates
                        .last_mut()
                        .filter(|certificate| certificate.fingerprint.is_empty())
                    {
                        field(line, 9).clone_into(&mut certificate.fingerprint);
                    }
                }
                "uid" => {
                    if let Some(certificate) = certificates.last_mut() {
                        let value = unescaped(field(line, 9));
                        match value.starts_with('<') {
                            true => certificate.emails.extend(email_of(&value)),
                            false if certificate.subject.is_empty() => certificate.subject = value,
                            false => {}
                        }
                    }
                }
                _ => {}
            }
        }
        certificates.retain(|certificate| !certificate.fingerprint.is_empty());
        Ok(certificates)
    }

    pub fn smime_certificates(&self, query: &str) -> Vec<Vec<u8>> {
        let mut command = self.command(GPGSM);
        command.args(["--export", query]);
        match self.run(command, None) {
            Ok(data) => split_certificates(&data),
            Err(error) => {
                tracing::warn!("cannot look up S/MIME certificates: {error}");
                Vec::new()
            }
        }
    }

    pub fn import_pkcs12(&self, file: &Path) -> Result<()> {
        let mut command = self.command(GPGSM);
        command.arg("--import").arg(file);
        self.run(command, None).map(drop)
    }
}

fn split_certificates(data: &[u8]) -> Vec<Vec<u8>> {
    let mut certificates = Vec::new();
    let mut rest = data;
    while rest.len() > 4 && rest[0] == 0x30 {
        let length = match rest[1] {
            0x82 => usize::from(rest[2]) << 8 | usize::from(rest[3]),
            0x83 if rest.len() > 4 => {
                usize::from(rest[2]) << 16 | usize::from(rest[3]) << 8 | usize::from(rest[4])
            }
            _ => break,
        };
        let total = length + 2 + usize::from(rest[1] & 0x7f);
        let Some((certificate, tail)) = rest.split_at_checked(total) else {
            break;
        };
        certificates.push(certificate.to_vec());
        rest = tail;
    }
    certificates
}
