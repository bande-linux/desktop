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
