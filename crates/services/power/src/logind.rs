use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

pub use bande_bus::logind::{
    ManagerProxyBlocking, SessionProxyBlocking, follow_blocked, follow_lock_requests,
    follow_locked_hint, hibernates_after_sleep, inhibit, logind_sets, own_session,
    runs_in_virtual_machine, sleep,
};
use bande_l10n::tr;
use bande_notify::Urgency;
use bande_notify::client::{self, Request};
use rustix::time::{ClockId, clock_gettime};
use zbus::blocking::Connection;

const SLEPT_AT_LEAST: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sleep {
    Starting,
    Woke,
    Failed,
}

fn time_asleep_since_boot() -> Option<Duration> {
    let boot = Duration::try_from(clock_gettime(ClockId::Boottime)).ok()?;
    let awake = Duration::try_from(clock_gettime(ClockId::Monotonic)).ok()?;
    boot.checked_sub(awake)
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
    pub fn start(
        system: &Connection,
        lock_wait: Duration,
        sleeping: impl Fn(Sleep) + Send + 'static,
    ) -> zbus::Result<Self> {
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
                    &tr!("Lock the screen before the computer sleeps"),
                    "delay",
                )
            }
        };
        let mut held = Some(take()?);
        let mut asleep_before = None;
        let signals = manager.receive_prepare_for_sleep()?;
        thread::spawn(move || {
            for signal in signals {
                let Ok(arguments) = signal.args() else {
                    continue;
                };
                let start = *arguments.start();
                if start {
                    asleep_before = time_asleep_since_boot();
                    sleeping(Sleep::Starting);
                    lock_and_wait(&session, &hint, lock_wait);
                    held.take();
                    continue;
                }
                let slept = asleep_before
                    .take()
                    .zip(time_asleep_since_boot())
                    .is_none_or(|(before, after)| after.saturating_sub(before) >= SLEPT_AT_LEAST);
                sleeping(match slept {
                    true => Sleep::Woke,
                    false => Sleep::Failed,
                });
                if held.is_none() {
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

pub fn report_failed_sleep(session: &Connection) {
    tracing::warn!("the computer did not sleep");
    let app_name = tr!("Power");
    let summary = tr!("The Computer Didn't Sleep");
    let body = tr!("Something on this computer stopped it from sleeping, so it kept running.");
    let posted = client::notify(
        session,
        &Request {
            app_name: &app_name,
            replaces_id: 0,
            app_icon: "battery-caution",
            summary: &summary,
            body: &body,
            actions: &[],
            urgency: Urgency::Normal,
            sound: None,
            desktop_entry: None,
        },
    );
    if let Err(error) = posted {
        tracing::warn!("cannot report that the computer did not sleep: {error}");
    }
}
