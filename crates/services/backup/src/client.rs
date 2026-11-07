use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{OwnedValue, Type, Value};
use zbus::{MatchRule, message};

pub const BUS_NAME: &str = "org.bande.Backup";
pub const OBJECT_PATH: &str = "/org/bande/Backup";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type, Value, OwnedValue)]
pub struct Status {
    pub phase: String,
    pub problem: String,
    pub setup_error: String,
    pub disk_name: String,
    pub on_network: bool,
    pub excluded: Vec<String>,
    pub last_success: u64,
    pub paused_until: u64,
    pub oldest: u64,
    pub versions: u32,
    pub percent: f64,
    pub remaining_seconds: u64,
    pub disk_free: u64,
    pub disk_total: u64,
    pub on_battery: bool,
    pub browse_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DiskChoice {
    pub uuid: String,
    pub name: String,
    pub filesystem: String,
    pub size: u64,
    pub block: String,
    pub drive_name: String,
    pub drive_size: u64,
    pub contents: String,
}

pub mod phase {
    pub const NO_DISK: &str = "no-disk";
    pub const IDLE: &str = "idle";
    pub const PREPARING: &str = "preparing";
    pub const BACKING_UP: &str = "backing-up";
    pub const TIDYING: &str = "tidying";
    pub const VERIFYING: &str = "verifying";
    pub const PAUSED: &str = "paused";
    pub const DISK_AWAY: &str = "disk-away";
    pub const WAITING_FOR_POWER: &str = "waiting-for-power";
}

pub mod problem {
    pub const OVERDUE: &str = "overdue";
    pub const DISK_FULL: &str = "disk-full";
    pub const DAMAGED: &str = "damaged";
    pub const CANNOT_OPEN: &str = "cannot-open";
    pub const FAILING: &str = "failing";
}

pub mod setup_error {
    pub const WRONG_PASSWORD: &str = "wrong-password";
    pub const DISK_GONE: &str = "disk-gone";
    pub const FAILED: &str = "failed";
    pub const ERASE_FAILED: &str = "erase-failed";
    pub const UNREACHABLE: &str = "unreachable";
    pub const BAD_ADDRESS: &str = "bad-address";
}

#[zbus::proxy(
    interface = "org.bande.Backup",
    default_service = "org.bande.Backup",
    default_path = "/org/bande/Backup",
    gen_async = false,
    blocking_name = "BackupProxy"
)]
pub trait Backup {
    #[zbus(property(emits_changed_signal = "false"))]
    fn status(&self) -> zbus::Result<Status>;

    fn disks(&self) -> zbus::Result<Vec<DiskChoice>>;
    fn has_backups(&self, uuid: &str) -> zbus::Result<bool>;
    fn choose(&self, uuid: &str, password: &str) -> zbus::Result<()>;
    fn check_network(&self, url: &str, user: &str, share_password: &str) -> zbus::Result<String>;
    fn choose_network(
        &self,
        url: &str,
        user: &str,
        share_password: &str,
        password: &str,
    ) -> zbus::Result<()>;
    fn erase_and_choose(&self, block: &str, password: &str) -> zbus::Result<()>;
    fn set_excluded(&self, folders: Vec<String>) -> zbus::Result<()>;
    fn release(&self) -> zbus::Result<()>;
    fn stop_using(&self) -> zbus::Result<()>;
    fn back_up_now(&self) -> zbus::Result<()>;
    fn pause(&self) -> zbus::Result<()>;
    fn resume(&self) -> zbus::Result<()>;
    fn set_on_battery(&self, allowed: bool) -> zbus::Result<()>;
    fn open_browser(&self) -> zbus::Result<()>;
    fn close_browser(&self) -> zbus::Result<()>;
}

fn relay(
    rule: MatchRule<'static>,
    system: &Connection,
    sender: mpsc::Sender<()>,
    stop_when_absent: bool,
) -> zbus::Result<()> {
    let signals = MessageIterator::for_match_rule(rule, system, None)?;
    thread::spawn(move || {
        for signal in signals.flatten() {
            if stop_when_absent {
                let owner = signal
                    .body()
                    .deserialize::<(String, String, String)>()
                    .map(|(_, _, new_owner)| new_owner);
                if owner.is_ok_and(|owner| owner.is_empty()) {
                    continue;
                }
            }
            if sender.send(()).is_err() {
                return;
            }
        }
    });
    Ok(())
}

pub fn follow_status(
    system: &Connection,
    mut on_status: impl FnMut(zbus::Result<Status>) -> bool + Send + 'static,
) -> zbus::Result<()> {
    let (sender, changes) = mpsc::channel();
    let changed = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .path(OBJECT_PATH)?
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .build();
    let owner = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg(0, BUS_NAME)?
        .build();
    relay(changed, system, sender.clone(), false)?;
    relay(owner, system, sender, true)?;
    let proxy = BackupProxy::new(system)?;
    thread::spawn(move || {
        if !on_status(proxy.status()) {
            return;
        }
        for () in changes {
            if !on_status(proxy.status()) {
                return;
            }
        }
    });
    Ok(())
}

pub const VERSIONS_ROOT: &str = "/run/bande-backup-browse/versions";
pub const LOCAL_VERSIONS_ROOT: &str = "/run/bande-backup-local/versions";
const LATEST: &str = "latest";
const OPEN_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CannotBrowse {
    NotInstalled,
    NoDisk,
    DiskAway,
    Failed,
}

pub struct Browsing {
    system: Connection,
}

impl Drop for Browsing {
    fn drop(&mut self) {
        let closed = BackupProxy::new(&self.system).and_then(|proxy| proxy.close_browser());
        if let Err(error) = closed {
            tracing::debug!("cannot close the backup browser: {error}");
        }
    }
}

pub fn open_browser() -> Result<Browsing, CannotBrowse> {
    let system = Connection::system().map_err(|_| CannotBrowse::Failed)?;
    let proxy = BackupProxy::new(&system).map_err(|_| CannotBrowse::Failed)?;
    let first = proxy.status().map_err(|_| CannotBrowse::NotInstalled)?;
    if first.phase == phase::NO_DISK {
        return Err(CannotBrowse::NoDisk);
    }
    let (sender, outcome) = mpsc::channel();
    let mut asked = false;
    follow_status(&system, move |status| {
        let Ok(status) = status else {
            return true;
        };
        let answer = if !status.browse_root.is_empty() {
            Some(Ok(()))
        } else if status.phase == phase::DISK_AWAY && asked {
            Some(Err(CannotBrowse::DiskAway))
        } else {
            None
        };
        asked = true;
        match answer {
            Some(answer) => {
                let _ = sender.send(answer);
                false
            }
            None => true,
        }
    })
    .map_err(|_| CannotBrowse::Failed)?;
    proxy.open_browser().map_err(|_| CannotBrowse::Failed)?;
    let browsing = Browsing { system };
    outcome
        .recv_timeout(OPEN_TIMEOUT)
        .map_err(|_| CannotBrowse::Failed)??;
    Ok(browsing)
}

fn times_in(root: &str) -> Vec<String> {
    fs::read_dir(root)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name != LATEST)
                .collect()
        })
        .unwrap_or_default()
}

pub fn version_times() -> Vec<String> {
    let mut times = times_in(VERSIONS_ROOT);
    times.extend(times_in(LOCAL_VERSIONS_ROOT));
    times.sort();
    times.dedup();
    times
}

pub fn is_version_path(path: &Path) -> bool {
    path.starts_with(VERSIONS_ROOT) || path.starts_with(LOCAL_VERSIONS_ROOT)
}

pub fn in_version(time: &str, live: &Path) -> PathBuf {
    let relative = live.strip_prefix("/").unwrap_or(live);
    let local = Path::new(LOCAL_VERSIONS_ROOT).join(time);
    if local.is_dir() {
        local.join(relative)
    } else {
        Path::new(VERSIONS_ROOT).join(time).join(relative)
    }
}

pub fn live_path(version_path: &Path) -> Option<(String, PathBuf)> {
    let rest = version_path
        .strip_prefix(VERSIONS_ROOT)
        .or_else(|_| version_path.strip_prefix(LOCAL_VERSIONS_ROOT))
        .ok()?;
    let mut components = rest.components();
    let time = components
        .next()?
        .as_os_str()
        .to_string_lossy()
        .into_owned();
    Some((time, Path::new("/").join(components.as_path())))
}
