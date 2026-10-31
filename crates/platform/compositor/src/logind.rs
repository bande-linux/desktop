use std::sync::mpsc::Receiver;
use std::thread;

use smithay::reexports::calloop::channel::{self, Channel};
use zbus::blocking::Connection;

const OWN_SESSION: &str = "/org/freedesktop/login1/session/auto";

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait Session {
    fn set_locked_hint(&self, locked: bool) -> zbus::Result<()>;

    #[zbus(signal)]
    fn lock(&self) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Manager {
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRequest {
    Lock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sleep {
    Starting,
    Ended,
}

pub fn follow_sleep() -> zbus::Result<Channel<Sleep>> {
    let connection = Connection::system()?;
    let changes = ManagerProxyBlocking::new(&connection)?.receive_prepare_for_sleep()?;
    let (sender, channel) = channel::channel();
    thread::spawn(move || {
        for change in changes {
            let Ok(arguments) = change.args() else {
                continue;
            };
            let sleep = if *arguments.start() {
                Sleep::Starting
            } else {
                Sleep::Ended
            };
            if sender.send(sleep).is_err() {
                return;
            }
        }
    });
    Ok(channel)
}

pub fn follow_session(lock_states: Receiver<bool>) -> zbus::Result<Channel<SessionRequest>> {
    let connection = Connection::system()?;
    let session = SessionProxyBlocking::builder(&connection)
        .path(OWN_SESSION)?
        .build()?;
    let locks = session.receive_lock()?;
    let (requests, channel) = channel::channel();
    thread::spawn(move || {
        for _ in locks {
            if requests.send(SessionRequest::Lock).is_err() {
                return;
            }
        }
    });
    thread::spawn(move || {
        for locked in lock_states {
            if let Err(error) = session.set_locked_hint(locked) {
                tracing::warn!("cannot tell logind the session is locked: {error}");
            }
        }
    });
    Ok(channel)
}
