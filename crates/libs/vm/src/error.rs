use std::fmt;
use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;

use bande_https::ureq;
use bande_l10n::tr;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperFailure {
    NoPkexec,
    NoHelper,
    NotAuthorized,
    Refused(String),
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Json(serde_json::Error),
    Bus(zbus::Error),
    Http(ureq::Error),
    Qmp {
        class: String,
        description: String,
    },
    Protocol(String),
    Checksum {
        expected: String,
        actual: String,
    },
    NotListed(String),
    NoRelease(String),
    NoSnapshot,
    NoShare,
    Tool {
        program: String,
        status: ExitStatus,
        stderr: String,
    },
    Unit {
        unit: String,
        result: String,
    },
    Missing(String),
    NotUtf8(PathBuf),
    Cancelled,
    Helper(HelperFailure),
    NeedsShutDown,
    Signature(String),
    HasSnapshots,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Json(error) => write!(f, "invalid JSON: {error}"),
            Self::Bus(error) => write!(f, "D-Bus: {error}"),
            Self::Http(error) => write!(f, "download: {error}"),
            Self::Qmp { class, description } => write!(f, "QEMU: {description} ({class})"),
            Self::Protocol(message) => write!(f, "QMP protocol: {message}"),
            Self::Checksum { expected, actual } => f.write_str(&tr!(
                "The download is damaged: its checksum is {actual}, not {expected}.",
                actual = actual.clone(),
                expected = expected.clone()
            )),
            Self::NotListed(file) => f.write_str(&tr!(
                "{file} is not in the vendor’s checksums.",
                file = file.clone()
            )),
            Self::NoRelease(os) => f.write_str(&tr!(
                "No {os} release is signed with a key Velox knows.",
                os = os.clone()
            )),
            Self::NoSnapshot => f.write_str(&tr!("This snapshot no longer exists.")),
            Self::NoShare => f.write_str(&tr!("This shared folder no longer exists.")),
            Self::Tool {
                program,
                status,
                stderr,
            } => write!(f, "{program} failed ({status}): {}", stderr.trim()),
            Self::Unit { unit, result } => write!(f, "{unit} did not start: {result}"),
            Self::Missing(what) => {
                f.write_str(&tr!("{what} is not available.", what = what.clone()))
            }
            Self::NotUtf8(path) => write!(f, "{} is not a UTF-8 path", path.display()),
            Self::Signature(message) => f.write_str(&tr!(
                "The vendor’s checksums are not signed correctly: {message}",
                message = message.clone()
            )),
            Self::NeedsShutDown => f.write_str(&tr!("Shut the machine down first.")),
            Self::HasSnapshots => f.write_str(&tr!(
                "A machine with snapshots can only be cloned in full."
            )),
            Self::Cancelled => f.write_str(&tr!("Canceled.")),
            Self::Helper(HelperFailure::NoPkexec) => f.write_str(&tr!(
                "Velox needs pkexec to ask for an administrator’s password. Install the pkexec package."
            )),
            Self::Helper(HelperFailure::NoHelper) => {
                f.write_str(&tr!("The Velox helper is missing. Reinstall Velox."))
            }
            Self::Helper(HelperFailure::NotAuthorized) => f.write_str(&tr!(
                "The administrator’s password was not given, or this account is not allowed to do this."
            )),
            Self::Helper(HelperFailure::Refused(reason)) => f.write_str(&tr!(
                "The Velox helper refused: {reason}",
                reason = reason.clone()
            )),
        }
    }
}

impl Error {
    /// Whether the text says what went wrong in terms the user can act on, rather than a technical detail for the log.
    pub fn explains(&self) -> bool {
        matches!(
            self,
            Self::Checksum { .. }
                | Self::NotListed(_)
                | Self::NoRelease(_)
                | Self::NoSnapshot
                | Self::NoShare
                | Self::Missing(_)
                | Self::Signature(_)
                | Self::NeedsShutDown
                | Self::HasSnapshots
                | Self::Cancelled
                | Self::Helper(_)
        )
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<zbus::Error> for Error {
    fn from(error: zbus::Error) -> Self {
        Self::Bus(error)
    }
}

impl From<zbus::fdo::Error> for Error {
    fn from(error: zbus::fdo::Error) -> Self {
        Self::Bus(error.into())
    }
}

impl From<ureq::Error> for Error {
    fn from(error: ureq::Error) -> Self {
        Self::Http(error)
    }
}

impl From<rustix::io::Errno> for Error {
    fn from(error: rustix::io::Errno) -> Self {
        Self::Io(error.into())
    }
}
