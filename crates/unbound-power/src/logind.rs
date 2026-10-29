use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;

const LOCK_POLL: Duration = Duration::from_millis(20);

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

pub struct SleepGuard {
    _connection: Connection,
}

impl SleepGuard {
    pub fn start(system: &Connection, lock_wait: Duration) -> zbus::Result<Self> {
        let manager = ManagerProxyBlocking::new(system)?;
        let session = own_session(system)?;
        let take = {
            let system = system.clone();
            move || {
                inhibit(
                    &system,
                    "sleep",
                    "Unbound",
                    "Lock the screen before the computer sleeps",
                    "delay",
                )
            }
        };
        let held = Arc::new(Mutex::new(Some(take()?)));
        let signals = manager.receive_prepare_for_sleep()?;
        thread::spawn(move || {
            for signal in signals {
                let Ok(arguments) = signal.args() else {
                    continue;
                };
                let mut held = held.lock().unwrap_or_else(PoisonError::into_inner);
                if *arguments.start() {
                    lock_and_wait(&session, lock_wait);
                    held.take();
                } else if held.is_none() {
                    match take() {
                        Ok(fd) => *held = Some(fd),
                        Err(error) => {
                            eprintln!("unbound-power: cannot delay sleep for locking: {error}")
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

fn lock_and_wait(session: &SessionProxyBlocking<'_>, lock_wait: Duration) {
    if session.locked_hint().unwrap_or(false) {
        return;
    }
    if let Err(error) = session.lock() {
        eprintln!("unbound-power: cannot lock the session before sleep: {error}");
        return;
    }
    let deadline = Instant::now() + lock_wait;
    while Instant::now() < deadline && !session.locked_hint().unwrap_or(false) {
        thread::sleep(LOCK_POLL);
    }
}
