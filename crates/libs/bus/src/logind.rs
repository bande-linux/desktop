use std::os::fd::OwnedFd;
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

fn runs_on_a_battery(system: &Connection) -> bool {
    PowerSourceProxyBlocking::new(system).is_ok_and(|source| {
        source.is_present().unwrap_or(false)
            && source.kind().is_ok_and(|kind| kind == UPOWER_BATTERY)
    })
}

pub fn hibernates_after_sleep(manager: &ManagerProxyBlocking<'_>) -> zbus::Result<bool> {
    Ok(runs_on_a_battery(manager.inner().connection())
        && manager.can_suspend_then_hibernate()? == "yes")
}

/// Puts the computer to sleep, hibernating before a battery runs out where logind can, and
/// answers whether logind allowed it.
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
