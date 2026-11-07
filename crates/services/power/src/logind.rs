use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

pub use bande_bus::logind::{
    ManagerProxyBlocking, SessionProxyBlocking, follow_blocked, follow_locked_hint, inhibit,
    own_session,
};
use zbus::blocking::Connection;

use crate::L10N;

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
