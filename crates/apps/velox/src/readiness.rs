use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use bande_l10n::{Text, text, tr};

use crate::engine::{Engine, EngineInfo, Error, Flavor};

const DOCKER_INSTALL: &str = "\
sudo apt install docker.io docker-compose docker-buildx
sudo usermod -aG docker $USER";
const PODMAN_INSTALL: &str = "\
sudo apt install podman
systemctl --user enable --now podman.socket";
const DOCKER_START: &str = "sudo systemctl enable --now docker";
const PODMAN_START: &str = "systemctl --user enable --now podman.socket";
const DOCKER_GROUP: &str = "sudo usermod -aG docker $USER";
const ACCESS_LINK: &str = "https://docs.docker.com/engine/install/linux-postinstall/";
const BINARY_FOLDERS: [&str; 4] = ["/usr/bin", "/usr/sbin", "/usr/local/bin", "/usr/local/sbin"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    NotInstalled,
    NotRunning(Flavor),
    NoAccess,
    Relogin,
    TooOld,
    Unreachable {
        host: String,
        reason: Option<String>,
    },
}

pub enum Probe {
    Ready(EngineInfo),
    Blocked(Readiness),
}

pub struct Steps {
    pub title: String,
    pub commands: &'static str,
}

pub struct Advice {
    pub headline: String,
    pub steps: Option<Steps>,
    pub note: Option<String>,
    pub link: Option<&'static str>,
    pub alternative: Option<Steps>,
}

fn podman_alternative() -> Steps {
    Steps {
        title: tr!("Or Podman, rootless"),
        commands: PODMAN_INSTALL,
    }
}

impl Readiness {
    pub fn advice(&self) -> Advice {
        let advice = |headline: Text| Advice {
            headline: headline.tr(),
            steps: None,
            note: None,
            link: None,
            alternative: None,
        };
        let steps = |title: Text, commands| {
            Some(Steps {
                title: title.tr(),
                commands,
            })
        };
        match self {
            Self::NotInstalled => Advice {
                steps: steps(text!("Podman, rootless (recommended)"), PODMAN_INSTALL),
                alternative: Some(Steps {
                    title: tr!("Or Docker Engine"),
                    commands: DOCKER_INSTALL,
                }),
                ..advice(text!("Install a container engine to continue"))
            },
            Self::NotRunning(Flavor::Docker) => Advice {
                steps: steps(text!("Start Docker"), DOCKER_START),
                ..advice(text!("Docker is not running"))
            },
            Self::NotRunning(Flavor::Podman) => Advice {
                steps: steps(text!("Start Podman"), PODMAN_START),
                ..advice(text!("Podman is not running"))
            },
            Self::NoAccess => Advice {
                steps: steps(
                    text!("Add yourself to the docker group, then log out and back in"),
                    DOCKER_GROUP,
                ),
                note: Some(tr!("The docker group is root-equivalent.")),
                link: Some(ACCESS_LINK),
                alternative: Some(podman_alternative()),
                ..advice(text!("No access to Docker"))
            },
            Self::Relogin => advice(text!("Log out and back in to finish")),
            Self::TooOld => advice(text!("Update Docker to continue")),
            Self::Unreachable { host, reason } => {
                let reason = reason.clone().filter(|reason| !reason.is_empty());
                let headline = match reason {
                    Some(_) => text!("Cannot reach {host}"),
                    None => text!("Connecting to {host} over SSH…"),
                };
                Advice {
                    headline: headline
                        .tr_args(&[("host", &host.clone() as &dyn std::fmt::Display)]),
                    steps: None,
                    note: reason,
                    link: None,
                    alternative: None,
                }
            }
        }
    }
}

pub(crate) fn binary_folders() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(BINARY_FOLDERS.iter().map(PathBuf::from))
        .collect()
}

fn installed(binary: &str) -> bool {
    binary_folders()
        .iter()
        .any(|folder| folder.join(binary).is_file())
}

fn status_field(name: &str) -> Vec<u32> {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find_map(|line| line.strip_prefix(name))
        .map(|values| {
            values
                .split_whitespace()
                .filter_map(|value| value.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn listed_in_group(gid: u32) -> bool {
    let Ok(user) = std::env::var("USER") else {
        return false;
    };
    std::fs::read_to_string("/etc/group")
        .unwrap_or_default()
        .lines()
        .map(|line| line.split(':').collect::<Vec<_>>())
        .any(|fields| {
            matches!(fields.as_slice(), [_, _, id, members]
                if id.parse() == Ok(gid) && members.split(',').any(|member| member == user))
        })
}

fn needs_relogin(socket: &Path) -> bool {
    let Ok(gid) = std::fs::metadata(socket).map(|metadata| metadata.gid()) else {
        return false;
    };
    let session = status_field("Groups:");
    let primary = status_field("Gid:");
    !session.contains(&gid) && !primary.contains(&gid) && listed_in_group(gid)
}

pub fn probe(candidates: &[PathBuf]) -> Probe {
    let mut too_old = false;
    let mut denied = None;
    for socket in candidates
        .iter()
        .filter(|socket| socket.symlink_metadata().is_ok())
    {
        match Engine::new(socket).info() {
            Ok(info) if info.supported => return Probe::Ready(info),
            Ok(_) => too_old = true,
            Err(Error::Io(error)) if error.kind() == io::ErrorKind::PermissionDenied => {
                denied.get_or_insert(socket);
            }
            Err(error) => tracing::debug!("{} does not answer: {error}", socket.display()),
        }
    }
    Probe::Blocked(match (too_old, denied) {
        (true, _) => Readiness::TooOld,
        (false, Some(socket)) if needs_relogin(socket) => Readiness::Relogin,
        (false, Some(_)) => Readiness::NoAccess,
        _ if installed("dockerd") => Readiness::NotRunning(Flavor::Docker),
        _ if installed("podman") => Readiness::NotRunning(Flavor::Podman),
        _ => Readiness::NotInstalled,
    })
}
