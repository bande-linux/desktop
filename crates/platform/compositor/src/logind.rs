use std::os::fd::OwnedFd;
use std::sync::mpsc::Receiver;
use std::thread;

use bande_l10n::tr;
use smithay::reexports::calloop::channel::Sender;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;

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

    fn get_session(&self, session_id: &str) -> zbus::Result<OwnedObjectPath>;

    fn can_suspend(&self) -> zbus::Result<String>;

    fn suspend(&self, interactive: bool) -> zbus::Result<()>;

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

pub fn follow_sleep(connection: &Connection, sender: Sender<Sleep>) -> zbus::Result<()> {
    let changes = ManagerProxyBlocking::new(connection)?.receive_prepare_for_sleep()?;
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
    Ok(())
}

pub fn sleep_on_request(connection: &Connection, requests: Receiver<()>) -> zbus::Result<()> {
    let manager = ManagerProxyBlocking::new(connection)?;
    thread::spawn(move || {
        for () in requests {
            let slept = manager
                .can_suspend()
                .and_then(|answer| match answer.as_str() {
                    "yes" => manager.suspend(false),
                    _ => Ok(()),
                });
            if let Err(error) = slept {
                tracing::warn!("cannot sleep on the power button: {error}");
            }
        }
    });
    Ok(())
}

pub fn follow_shutdown(connection: &Connection, sender: Sender<OwnedFd>) -> zbus::Result<()> {
    let manager = ManagerProxyBlocking::new(connection)?;
    let changes = manager.receive_prepare_for_shutdown()?;
    let take = move || -> zbus::Result<OwnedFd> {
        let delay = manager.inhibit(
            "shutdown",
            "Bande",
            &tr!("Remember the open apps to reopen them at the next login"),
            "delay",
        )?;
        Ok(delay.into())
    };
    let mut held = Some(take()?);
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
    Ok(())
}

pub fn follow_session(
    connection: &Connection,
    lock_states: Receiver<bool>,
    requests: Sender<SessionRequest>,
) -> zbus::Result<()> {
    let path = ManagerProxyBlocking::new(connection)?.get_session("auto")?;
    let session = SessionProxyBlocking::builder(connection)
        .path(path)?
        .build()?;
    let locks = session.receive_lock()?;
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
    Ok(())
}
