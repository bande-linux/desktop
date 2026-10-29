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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRequest {
    Lock,
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
