pub mod offline;
mod transaction;

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use zbus::blocking::Connection;
use zbus::blocking::fdo::DBusProxy;
use zbus::names::BusName;
use zbus::zvariant::OwnedValue;

pub use transaction::Canceller;
use transaction::Outcome;

pub const SERVICE: &str = "org.freedesktop.PackageKit";
pub const PATH: &str = "/org/freedesktop/PackageKit";

const FILTER_NONE: u64 = 1 << 1;
const FILTER_INSTALLED: u64 = 1 << 2;
const ONLY_TRUSTED: u64 = 1 << 1;
const SIMULATE: u64 = 1 << 2;
const ONLY_DOWNLOAD: u64 = 1 << 3;
const ROLE_REFRESH_CACHE: u32 = 13;
const INFO_MASK: u32 = 0xffff;
const OS_RELEASE: &str = "/proc/sys/kernel/osrelease";
const OFFLINE_PACKAGES: [&str; 4] = ["libc6", "systemd", "libsystemd0", "udev"];
const KERNEL_PACKAGES: [&str; 4] = [
    "linux-image-",
    "linux-modules-",
    "linux-binary-",
    "linux-base-",
];
const BOOT_PACKAGES: [&str; 5] = [
    "firmware-",
    "initramfs-tools",
    "u-boot",
    "grub",
    "systemd-boot",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Info {
    Installed,
    Available,
    Security,
    Blocked,
    Updating,
    Installing,
    Removing,
    Obsoleting,
    Reinstalling,
    Downgrading,
    Other(u32),
}

impl Info {
    fn from_raw(raw: u32) -> Self {
        match raw & INFO_MASK {
            1 => Self::Installed,
            2 => Self::Available,
            8 => Self::Security,
            9 => Self::Blocked,
            11 => Self::Updating,
            12 => Self::Installing,
            13 => Self::Removing,
            15 => Self::Obsoleting,
            19 => Self::Reinstalling,
            20 => Self::Downgrading,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub id: String,
    pub name: String,
    pub version: String,
    pub arch: String,
    pub origin: String,
    pub info: Info,
    pub summary: String,
}

impl Package {
    pub fn new(info: Info, id: &str, summary: &str) -> Self {
        let mut parts = id.split(';').map(str::to_owned);
        let mut next = || parts.next().unwrap_or_default();
        Self {
            id: id.to_owned(),
            name: next(),
            version: next(),
            arch: next(),
            origin: next(),
            info,
            summary: summary.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Restart {
    #[default]
    None,
    Application,
    Session,
    System,
    SecuritySession,
    SecuritySystem,
}

impl Restart {
    fn from_raw(raw: u32) -> Self {
        match raw {
            2 => Self::Application,
            3 => Self::Session,
            4 => Self::System,
            5 => Self::SecuritySession,
            6 => Self::SecuritySystem,
            _ => Self::None,
        }
    }

    pub fn reboots(self) -> bool {
        matches!(self, Self::System | Self::SecuritySystem)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Class {
    #[default]
    Live,
    Boot,
    Offline,
}

impl Class {
    pub fn of(package: &Package, restart: Restart, release: &str) -> Self {
        let name = package.name.as_str();
        let kernel = KERNEL_PACKAGES
            .iter()
            .find_map(|prefix| name.strip_prefix(prefix));
        let login = name == "greetd" || name.starts_with("libpam");
        let core = OFFLINE_PACKAGES.contains(&name) || name.starts_with("dbus");
        if core
            || kernel == Some(release)
            || (login && package.info == Info::Security)
            || (restart.reboots() && kernel.is_none())
        {
            Self::Offline
        } else if kernel.is_some() || BOOT_PACKAGES.iter().any(|prefix| name.starts_with(prefix)) {
            Self::Boot
        } else {
            Self::Live
        }
    }
}

pub fn running_release() -> String {
    std::fs::read_to_string(OS_RELEASE)
        .map(|release| release.trim().to_owned())
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateDetail {
    pub id: String,
    pub restart: Restart,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Details {
    pub id: String,
    pub summary: String,
    pub description: String,
    pub url: String,
    pub size: u64,
}

impl Details {
    fn from_map(details: &HashMap<String, OwnedValue>) -> Self {
        let text = |key: &str| {
            details
                .get(key)
                .and_then(|value| String::try_from(value.try_clone().ok()?).ok())
                .unwrap_or_default()
        };
        let size = ["download-size", "size"]
            .iter()
            .filter_map(|key| u64::try_from(details.get(*key)?).ok())
            .find(|size| *size > 0)
            .unwrap_or(0);
        Self {
            id: text("package-id"),
            summary: text("summary"),
            description: text("description"),
            url: text("url"),
            size,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    NoNetwork,
    NotFound,
    AlreadyInstalled,
    Dependencies,
    Cancelled,
    SystemPackage,
    Locked,
    LocalInstallFailed,
    InvalidFile,
    IncompatibleArchitecture,
    NoSpace,
    NotAuthorized,
    Unsigned,
    Other(u32),
}

impl ErrorCode {
    fn from_raw(raw: u32) -> Self {
        match raw {
            2 => Self::NoNetwork,
            8 => Self::NotFound,
            9 => Self::AlreadyInstalled,
            13 => Self::Dependencies,
            17 => Self::Cancelled,
            20 => Self::SystemPackage,
            26 => Self::Locked,
            29 => Self::LocalInstallFailed,
            38 => Self::InvalidFile,
            45 => Self::IncompatibleArchitecture,
            46 => Self::NoSpace,
            48 => Self::NotAuthorized,
            50 => Self::Unsigned,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub code: ErrorCode,
    pub details: String,
}

#[derive(Debug)]
pub enum Error {
    Missing,
    Vanished,
    Failed(Failure),
    Bus(zbus::Error),
}

impl From<zbus::Error> for Error {
    fn from(error: zbus::Error) -> Self {
        let name = match &error {
            zbus::Error::MethodError(name, _, _) => name.as_str().to_owned(),
            zbus::Error::FDO(error) => zbus::DBusError::name(error.as_ref()).to_string(),
            _ => String::new(),
        };
        if name.ends_with(".ServiceUnknown") {
            return Self::Missing;
        }
        Self::Bus(error)
    }
}

impl From<zbus::fdo::Error> for Error {
    fn from(error: zbus::fdo::Error) -> Self {
        Self::from(zbus::Error::from(error))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(f, "PackageKit is not installed"),
            Self::Vanished => write!(f, "PackageKit stopped"),
            Self::Failed(failure) => write!(f, "{:?}: {}", failure.code, failure.details),
            Self::Bus(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Waiting,
    Downloading,
    Installing,
    Removing,
    Working,
}

impl Status {
    fn from_raw(raw: u32) -> Self {
        match raw {
            0..=2 | 30 | 31 => Self::Waiting,
            8 | 20..=25 => Self::Downloading,
            9 | 10 | 16 | 36 => Self::Installing,
            6 | 12 => Self::Removing,
            _ => Self::Working,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Progress {
    pub percent: Option<u32>,
    pub status: Status,
    pub allow_cancel: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Simulate,
    Apply,
}

impl Mode {
    fn flags(self) -> u64 {
        match self {
            Self::Simulate => SIMULATE,
            Self::Apply => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Applied {
    pub packages: Vec<Package>,
    pub restart: bool,
}

pub fn native<'a>(packages: &'a [Package], name: &str, arch: &str) -> Option<&'a Package> {
    let named = || packages.iter().filter(|package| package.name == name);
    named()
        .find(|package| package.arch == arch)
        .or_else(|| named().find(|package| package.arch == "all"))
}

pub fn debian_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        "arm" => "armhf",
        "x86" => "i386",
        other => other,
    }
}

pub fn available(system: &Connection) -> zbus::Result<bool> {
    let bus = DBusProxy::new(system)?;
    let name = BusName::try_from(SERVICE)?;
    Ok(bus.name_has_owner(name)?
        || bus
            .list_activatable_names()?
            .iter()
            .any(|activatable| activatable.as_str() == SERVICE))
}

#[zbus::proxy(
    interface = "org.freedesktop.PackageKit",
    default_service = "org.freedesktop.PackageKit",
    default_path = "/org/freedesktop/PackageKit"
)]
trait PackageKit {
    fn create_transaction(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
    fn get_time_since_action(&self, role: u32) -> zbus::Result<u32>;
}

#[derive(Clone)]
pub struct Client {
    system: Connection,
    hints: Vec<String>,
    canceller: Canceller,
}

impl Client {
    pub fn new(system: &Connection) -> Self {
        Self {
            system: system.clone(),
            hints: vec!["interactive=true".to_owned()],
            canceller: Canceller::default(),
        }
    }

    pub fn background(system: &Connection) -> Self {
        Self {
            hints: vec!["interactive=false".to_owned(), "background=true".to_owned()],
            ..Self::new(system)
        }
    }

    pub fn canceller(&self) -> Canceller {
        self.canceller.clone()
    }

    fn run<B>(&self, method: &str, body: &B, progress: &dyn Fn(Progress)) -> Result<Outcome, Error>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let outcome = transaction::run(self, method, body, progress)?;
        match outcome.failure.clone() {
            Some(failure) => Err(Error::Failed(failure)),
            None => Ok(outcome),
        }
    }

    pub fn refresh(&self, force: bool) -> Result<(), Error> {
        self.run("RefreshCache", &(force,), &|_| {}).map(drop)
    }

    pub fn seconds_since_refresh(&self) -> Result<u32, Error> {
        Ok(
            PackageKitProxyBlocking::new(&self.system)?
                .get_time_since_action(ROLE_REFRESH_CACHE)?,
        )
    }

    pub fn updates(&self) -> Result<Vec<Package>, Error> {
        let outcome = self.run("GetUpdates", &(FILTER_NONE,), &|_| {})?;
        Ok(outcome
            .packages
            .into_iter()
            .filter(|package| package.info != Info::Blocked)
            .collect())
    }

    pub fn details(&self, ids: &[String]) -> Result<Vec<Details>, Error> {
        Ok(self.run("GetDetails", &(ids,), &|_| {})?.details)
    }

    pub fn update_details(&self, ids: &[String]) -> Result<Vec<UpdateDetail>, Error> {
        Ok(self
            .run("GetUpdateDetail", &(ids,), &|_| {})?
            .update_details)
    }

    pub fn update(
        &self,
        ids: &[String],
        mode: Mode,
        progress: &dyn Fn(Progress),
    ) -> Result<Applied, Error> {
        Ok(self
            .run(
                "UpdatePackages",
                &(ONLY_TRUSTED | mode.flags(), ids),
                progress,
            )?
            .applied())
    }

    pub fn repair(&self, progress: &dyn Fn(Progress)) -> Result<(), Error> {
        self.run("RepairSystem", &(ONLY_TRUSTED,), progress)
            .map(drop)
    }

    pub fn download(&self, ids: &[String], progress: &dyn Fn(Progress)) -> Result<(), Error> {
        self.run(
            "UpdatePackages",
            &(ONLY_TRUSTED | ONLY_DOWNLOAD, ids),
            progress,
        )
        .map(drop)
    }

    pub fn resolve(&self, names: &[String]) -> Result<Vec<Package>, Error> {
        Ok(self
            .run("Resolve", &(FILTER_NONE, names), &|_| {})?
            .packages)
    }

    pub fn install(&self, ids: &[String], progress: &dyn Fn(Progress)) -> Result<Applied, Error> {
        Ok(self
            .run("InstallPackages", &(ONLY_TRUSTED, ids), progress)?
            .applied())
    }

    pub fn resolve_installed(&self, name: &str) -> Result<Vec<Package>, Error> {
        Ok(self
            .run(
                "Resolve",
                &(FILTER_INSTALLED, std::slice::from_ref(&name)),
                &|_| {},
            )?
            .packages)
    }

    pub fn files_local(&self, path: &Path) -> Result<Vec<String>, Error> {
        let file = path.to_string_lossy().into_owned();
        Ok(self
            .run("GetFilesLocal", &(std::slice::from_ref(&file),), &|_| {})?
            .files)
    }

    pub fn install_files(
        &self,
        path: &Path,
        mode: Mode,
        progress: &dyn Fn(Progress),
    ) -> Result<Applied, Error> {
        let file = path.to_string_lossy().into_owned();
        Ok(self
            .run(
                "InstallFiles",
                &(mode.flags(), std::slice::from_ref(&file)),
                progress,
            )?
            .applied())
    }

    pub fn remove(
        &self,
        ids: &[String],
        mode: Mode,
        progress: &dyn Fn(Progress),
    ) -> Result<Applied, Error> {
        Ok(self
            .run(
                "RemovePackages",
                &(mode.flags(), ids, true, false),
                progress,
            )?
            .applied())
    }
}
