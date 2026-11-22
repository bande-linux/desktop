use std::fs;
use std::io;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::thread;

use zbus::blocking::Connection;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
pub trait Manager {
    fn inhibit(
        &self,
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
    ) -> zbus::Result<zbus::zvariant::OwnedFd>;

    fn get_session(&self, session_id: &str) -> zbus::Result<OwnedObjectPath>;

    fn can_suspend(&self) -> zbus::Result<String>;

    fn suspend(&self, interactive: bool) -> zbus::Result<()>;

    fn can_suspend_then_hibernate(&self) -> zbus::Result<String>;

    fn suspend_then_hibernate(&self, interactive: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn block_inhibited(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn lid_closed(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn docked(&self) -> zbus::Result<bool>;

    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
pub trait Session {
    fn lock(&self) -> zbus::Result<()>;

    fn set_idle_hint(&self, idle: bool) -> zbus::Result<()>;

    fn set_brightness(&self, subsystem: &str, name: &str, brightness: u32) -> zbus::Result<()>;

    #[zbus(property)]
    fn locked_hint(&self) -> zbus::Result<bool>;

    #[zbus(signal, name = "Lock")]
    fn lock_requested(&self) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
trait UPower {
    #[zbus(property)]
    fn on_battery(&self) -> zbus::Result<bool>;
}

fn runs_on_battery(system: &Connection) -> bool {
    UPowerProxyBlocking::builder(system)
        .cache_properties(CacheProperties::No)
        .build()
        .and_then(|upower| upower.on_battery())
        .unwrap_or(false)
}

/// Answers whether a hibernated system would resume: the kernel knows its resume device (from
/// `resume=` or the initramfs) and the command line does not say `noresume`.
pub fn resume_configured() -> bool {
    let read = |path: &str| {
        fs::read_to_string(path).unwrap_or_else(|error| {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!("cannot read {path}: {error}");
            }
            String::new()
        })
    };
    resumes(&read("/proc/cmdline"), &read("/sys/power/resume"))
}

fn resumes(cmdline: &str, device: &str) -> bool {
    let mut words = cmdline.split_whitespace();
    if words.clone().any(|word| word == "noresume") {
        return false;
    }
    let named = words.any(|word| {
        word.strip_prefix("resume=")
            .is_some_and(|device| !device.is_empty() && device != "none")
    });
    named || !matches!(device.trim(), "" | "0:0")
}

pub fn hibernates_after_sleep(manager: &ManagerProxyBlocking<'_>) -> zbus::Result<bool> {
    Ok(runs_on_battery(manager.inner().connection())
        && manager.can_suspend_then_hibernate()? == "yes"
        && resume_configured())
}

/// Puts the computer to sleep, hibernating later on battery where it would resume, and answers
/// whether logind allowed it.
pub fn sleep(manager: &ManagerProxyBlocking<'_>, interactive: bool) -> zbus::Result<bool> {
    if hibernates_after_sleep(manager)? {
        manager.suspend_then_hibernate(interactive)?;
        return Ok(true);
    }
    if !interactive && manager.can_suspend()? != "yes" {
        return Ok(false);
    }
    manager.suspend(interactive)?;
    Ok(true)
}

pub fn own_session(system: &Connection) -> zbus::Result<SessionProxyBlocking<'static>> {
    let path = ManagerProxyBlocking::new(system)?.get_session("auto")?;
    SessionProxyBlocking::builder(system)
        .path(path)?
        .cache_properties(CacheProperties::No)
        .build()
}

pub fn inhibit(
    system: &Connection,
    what: &str,
    who: &str,
    why: &str,
    mode: &str,
) -> zbus::Result<OwnedFd> {
    let fd = ManagerProxyBlocking::new(system)?.inhibit(what, who, why, mode)?;
    Ok(fd.into())
}

pub fn follow_locked_hint(
    system: &Connection,
    changed: impl Fn(bool) + Send + 'static,
) -> zbus::Result<()> {
    let path = ManagerProxyBlocking::new(system)?.get_session("auto")?;
    let session = SessionProxyBlocking::builder(system).path(path)?.build()?;
    let changes = session.receive_locked_hint_changed();
    changed(session.locked_hint()?);
    thread::spawn(move || {
        for change in changes {
            if let Ok(locked) = change.get() {
                changed(locked);
            }
        }
    });
    Ok(())
}

pub fn follow_lock_requests(
    system: &Connection,
    requested: impl Fn() + Send + 'static,
) -> zbus::Result<()> {
    let path = ManagerProxyBlocking::new(system)?.get_session("auto")?;
    let session = SessionProxyBlocking::builder(system).path(path)?.build()?;
    let requests = session.receive_lock_requested()?;
    thread::spawn(move || {
        for _ in requests {
            requested();
        }
    });
    Ok(())
}

pub fn follow_blocked(
    system: &Connection,
    operation: &'static str,
    changed: impl Fn(bool) + Send + 'static,
) -> zbus::Result<()> {
    let manager = ManagerProxyBlocking::new(system)?;
    let blocks = move |what: &str| what.split(':').any(|what| what == operation);
    let changes = manager.receive_block_inhibited_changed();
    changed(blocks(&manager.block_inhibited()?));
    thread::spawn(move || {
        for change in changes {
            if let Ok(what) = change.get() {
                changed(blocks(&what));
            }
        }
    });
    Ok(())
}

pub fn runs_in_virtual_machine(system: &Connection) -> bool {
    crate::systemd::ServiceManagerProxyBlocking::new(system)
        .and_then(|manager| manager.virtualization())
        .is_ok_and(|virtualization| !virtualization.is_empty())
}

const LOGIND_CONFIG_FOLDERS: [&str; 4] = [
    "/etc/systemd",
    "/run/systemd",
    "/usr/local/lib/systemd",
    "/usr/lib/systemd",
];

fn logind_files(folder: &Path) -> Vec<PathBuf> {
    let mut files = vec![folder.join("logind.conf")];
    match fs::read_dir(folder.join("logind.conf.d")) {
        Ok(entries) => files.extend(
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "conf")
                }),
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!("cannot read {}/logind.conf.d: {error}", folder.display()),
    }
    files
}

/// Answers whether the administrator's logind configuration sets a key starting with `prefix`.
pub fn logind_sets(prefix: &str) -> bool {
    LOGIND_CONFIG_FOLDERS
        .iter()
        .flat_map(|folder| logind_files(Path::new(folder)))
        .any(|file| match fs::read_to_string(&file) {
            Ok(text) => text
                .lines()
                .any(|line| line.trim_start().starts_with(prefix)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => {
                tracing::warn!("cannot read {}: {error}", file.display());
                false
            }
        })
}

#[cfg(test)]
mod tests {
    use super::resumes;

    #[test]
    fn hibernation_resumes_only_from_a_named_or_known_resume_device() {
        assert!(resumes("root=/dev/sda2 resume=UUID=1234 quiet", "0:0\n"));
        assert!(resumes("root=/dev/sda2 quiet", "259:3\n"));
        assert!(!resumes("root=/dev/sda2 quiet", "0:0\n"));
        assert!(!resumes("root=/dev/sda2 resume=none", "0:0\n"));
        assert!(!resumes("resume=/dev/sda3 noresume", "8:3\n"));
    }
}
