use std::os::fd::OwnedFd;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;

use crate::L10N;

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

    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
pub trait Session {
    fn lock(&self) -> zbus::Result<()>;

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

#[derive(Default)]
struct LockedHint {
    locked: Mutex<bool>,
    changed: Condvar,
}

impl LockedHint {
    fn set(&self, locked: bool) {
        *self.locked.lock().unwrap_or_else(PoisonError::into_inner) = locked;
        self.changed.notify_all();
    }

    fn is_set(&self) -> bool {
        *self.locked.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait(&self, timeout: Duration) {
        let locked = self.locked.lock().unwrap_or_else(PoisonError::into_inner);
        drop(
            self.changed
                .wait_timeout_while(locked, timeout, |locked| !*locked)
                .unwrap_or_else(PoisonError::into_inner),
        );
    }
}

pub struct SleepGuard {
    _connection: Connection,
}

impl SleepGuard {
    pub fn start(system: &Connection, lock_wait: Duration) -> zbus::Result<Self> {
        let manager = ManagerProxyBlocking::new(system)?;
        let session = own_session(system)?;
        let hint = Arc::new(LockedHint::default());
        follow_locked_hint(system, {
            let hint = Arc::clone(&hint);
            move |locked| hint.set(locked)
        })?;
        let take = {
            let system = system.clone();
            move || {
                inhibit(
                    &system,
                    "sleep",
                    "Bande",
                    &L10N.tr("lock-before-sleep"),
                    "delay",
                )
            }
        };
        let mut held = Some(take()?);
        let signals = manager.receive_prepare_for_sleep()?;
        thread::spawn(move || {
            for signal in signals {
                let Ok(arguments) = signal.args() else {
                    continue;
                };
                if *arguments.start() {
                    lock_and_wait(&session, &hint, lock_wait);
                    held.take();
                } else if held.is_none() {
                    match take() {
                        Ok(fd) => held = Some(fd),
                        Err(error) => {
                            tracing::warn!("cannot delay sleep for locking: {error}")
                        }
                    }
                }
            }
        });
        Ok(Self {
            _connection: system.clone(),
        })
    }
}

fn lock_and_wait(session: &SessionProxyBlocking<'_>, hint: &LockedHint, lock_wait: Duration) {
    if hint.is_set() {
        return;
    }
    if let Err(error) = session.lock() {
        tracing::warn!("cannot lock the session before sleep: {error}");
        return;
    }
    hint.wait(lock_wait);
}
