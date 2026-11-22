use std::fs;
use std::io;
use std::os::fd::OwnedFd;
use std::thread;
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;

use crate::systemd_config;

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
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower/devices/DisplayDevice"
)]
trait PowerSource {
    #[zbus(property, name = "Type")]
    fn kind(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;
}

const UPOWER_BATTERY: u32 = 2;

fn has_a_battery(system: &Connection) -> bool {
    PowerSourceProxyBlocking::builder(system)
        .cache_properties(CacheProperties::No)
        .build()
        .is_ok_and(|source| {
            source.is_present().unwrap_or(false)
                && source.kind().is_ok_and(|kind| kind == UPOWER_BATTERY)
        })
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
    Ok(has_a_battery(manager.inner().connection())
        && manager.can_suspend_then_hibernate()? == "yes"
        && resume_configured())
}

/// Puts the computer to sleep, hibernating later on a computer with a battery where it would
/// resume, and answers whether logind allowed it.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HibernateDelay {
    pub after: Option<Duration>,
    pub on_ac_power: bool,
}

/// When `SuspendThenHibernate` hibernates, from systemd-sleep's effective configuration.
pub fn hibernate_delay() -> HibernateDelay {
    let sleep = |key: &str| systemd_config::setting("sleep.conf", key);
    HibernateDelay {
        after: sleep("HibernateDelaySec").and_then(|delay| systemd_config::timespan(&delay)),
        on_ac_power: sleep("HibernateOnACPower")
            .and_then(|value| systemd_config::boolean(&value))
            .unwrap_or(true),
    }
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
