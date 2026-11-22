use std::fs;
use std::io;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::Duration;

use bande_l10n::tr;
use smithay::reexports::calloop::channel::Sender;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait Session {
    #[zbus(property)]
    fn active(&self) -> zbus::Result<bool>;

    fn set_locked_hint(&self, locked: bool) -> zbus::Result<()>;

    #[zbus(name = "Lock")]
    fn request_lock(&self) -> zbus::Result<()>;

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

    #[zbus(property)]
    fn lid_closed(&self) -> zbus::Result<bool>;

    fn suspend(&self, interactive: bool) -> zbus::Result<()>;

    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;

    #[zbus(signal)]
    fn prepare_for_shutdown(&self, start: bool) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.RealtimeKit1",
    default_service = "org.freedesktop.RealtimeKit1",
    default_path = "/org/freedesktop/RealtimeKit1"
)]
trait RealtimeKit {
    fn make_thread_high_priority(&self, thread: u64, priority: i32) -> zbus::Result<()>;
}

const HIGH_PRIORITY_NICE_LEVEL: i32 = -11;

static NORMAL_NICE_LEVEL: OnceLock<i32> = OnceLock::new();

pub fn raise_priority(connection: &Connection) {
    let normal = match rustix::process::getpriority_process(None) {
        Ok(normal) => normal,
        Err(error) => {
            tracing::debug!("drawing and input keep the normal priority: {error}");
            return;
        }
    };
    let thread = rustix::thread::gettid().as_raw_nonzero().get();
    let raised = RealtimeKitProxyBlocking::new(connection).and_then(|rtkit| {
        rtkit.make_thread_high_priority(thread.unsigned_abs().into(), HIGH_PRIORITY_NICE_LEVEL)
    });
    match raised {
        Ok(()) => {
            NORMAL_NICE_LEVEL.get_or_init(|| normal);
            tracing::info!("drawing and input run at a higher priority");
        }
        Err(error) => tracing::debug!("drawing and input keep the normal priority: {error}"),
    }
}

pub fn normal_priority() -> io::Result<()> {
    match NORMAL_NICE_LEVEL.get() {
        Some(&nice) => rustix::process::setpriority_process(None, nice).map_err(io::Error::from),
        None => Ok(()),
    }
}

const LOGIND_CONFIG_FOLDERS: [&str; 4] = [
    "/etc/systemd",
    "/run/systemd",
    "/usr/local/lib/systemd",
    "/usr/lib/systemd",
];

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

const LID_SETTLES_AFTER_WAKE: Duration = Duration::from_secs(2);

pub fn follow_lid(connection: &Connection, sender: Sender<bool>) -> zbus::Result<()> {
    let manager = ManagerProxyBlocking::builder(connection)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()?;
    let wakes = manager.receive_prepare_for_sleep()?;
    let closed = manager.lid_closed()?;
    if sender.send(closed).is_err() {
        return Ok(());
    }
    thread::spawn(move || {
        for wake in wakes {
            if wake.args().is_ok_and(|arguments| *arguments.start()) {
                continue;
            }
            for wait in [Duration::ZERO, LID_SETTLES_AFTER_WAKE] {
                thread::sleep(wait);
                match manager.lid_closed() {
                    Ok(closed) => {
                        if sender.send(closed).is_err() {
                            return;
                        }
                    }
                    Err(error) => tracing::warn!("cannot tell whether the lid is closed: {error}"),
                }
            }
        }
    });
    Ok(())
}

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

fn power_key_configured() -> bool {
    LOGIND_CONFIG_FOLDERS
        .iter()
        .flat_map(|folder| logind_files(Path::new(folder)))
        .any(|file| match fs::read_to_string(&file) {
            Ok(text) => text
                .lines()
                .any(|line| line.trim_start().starts_with("HandlePowerKey=")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => {
                tracing::warn!("cannot read {}: {error}", file.display());
                false
            }
        })
}

pub fn handle_power_key(connection: &Connection, presses: Receiver<()>) -> zbus::Result<bool> {
    if power_key_configured() {
        return Ok(false);
    }
    let manager = ManagerProxyBlocking::new(connection)?;
    let session = SessionProxyBlocking::builder(connection)
        .path(manager.get_session("auto")?)?
        .build()?;
    let sleeper = bande_bus::logind::ManagerProxyBlocking::new(connection)?;
    let inhibitor = manager;
    let inhibit = move || {
        inhibitor
            .inhibit(
                "handle-power-key",
                "Bande",
                &tr!("The power button locks the screen and puts the computer to sleep"),
                "block",
            )
            .inspect_err(|error| tracing::warn!("cannot handle the power button: {error}"))
            .ok()
    };
    let changes = session.receive_active_changed();
    let mut held = match session.active()? {
        true => inhibit(),
        false => None,
    };
    thread::spawn(move || {
        for change in changes {
            match change.get() {
                Ok(true) if held.is_none() => held = inhibit(),
                Ok(true) => {}
                Ok(false) => held = None,
                Err(error) => tracing::warn!("cannot tell whether the session is active: {error}"),
            }
        }
    });
    thread::spawn(move || {
        for () in presses {
            if let Err(error) = session.request_lock() {
                tracing::warn!("cannot lock on the power button: {error}");
            }
            match bande_bus::logind::sleep(&sleeper, false) {
                Ok(true) => {}
                Ok(false) => tracing::info!("logind does not let the computer sleep now"),
                Err(error) => tracing::warn!("cannot sleep on the power button: {error}"),
            }
        }
    });
    Ok(true)
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
