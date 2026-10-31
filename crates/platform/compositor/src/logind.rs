use std::os::fd::OwnedFd;
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
    fn inhibit(
        &self,
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
    ) -> zbus::Result<zbus::zvariant::OwnedFd>;

    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;

    #[zbus(signal)]
    fn prepare_for_shutdown(&self, start: bool) -> zbus::Result<()>;
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

pub fn follow_shutdown() -> zbus::Result<Channel<OwnedFd>> {
    let connection = Connection::system()?;
    let manager = ManagerProxyBlocking::new(&connection)?;
    let changes = manager.receive_prepare_for_shutdown()?;
    let take = move || -> zbus::Result<OwnedFd> {
        let delay = manager.inhibit(
            "shutdown",
            "Unbound",
            &crate::L10N.tr("remember-apps-before-shutdown"),
            "delay",
        )?;
        Ok(delay.into())
    };
    let mut held = Some(take()?);
    let (sender, channel) = channel::channel();
    thread::spawn(move || {
        for change in changes {
            let Ok(arguments) = change.args() else {
                continue;
            };
            if *arguments.start() {
                if let Some(delay) = held.take()
                    && sender.send(delay).is_err()
                {
                    return;
                }
            } else if held.is_none() {
                held = take()
                    .inspect_err(|error| {
                        tracing::warn!("cannot delay shutdown to remember the apps: {error}")
                    })
                    .ok();
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
