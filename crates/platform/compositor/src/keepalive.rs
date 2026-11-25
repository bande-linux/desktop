use std::ffi::OsString;
use std::io;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use rustix::time::{
    Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, timerfd_create,
    timerfd_settime,
};

use crate::environment::notify_service_manager;
use crate::progress::waits_for_a_device;

const LOOK: Duration = Duration::from_secs(1);

/// How long waiting for a device, such as a busy disk, may keep the compositor from working
/// before it counts as hung.
pub(crate) const LONGEST_DEVICE_WAIT: Duration = Duration::from_secs(120);

struct Shared {
    socket: OsString,
    timer: OwnedFd,
    alive_at: Mutex<Instant>,
    late_after: Duration,
}

/// Reassures the service manager's watchdog: from the event loop, and while that is late only as
/// long as one of the compositor's threads waits for a device.
pub struct Keepalive(Arc<Shared>);

impl Shared {
    fn reassure(&self) {
        if let Err(error) = notify_service_manager(Some(&self.socket), "WATCHDOG=1") {
            tracing::warn!("cannot reassure the service manager's watchdog: {error}");
        }
    }

    fn look_in(&self, after: Duration) -> io::Result<()> {
        let none = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let timer = Itimerspec {
            it_interval: none,
            it_value: Timespec::try_from(after).map_err(io::Error::other)?,
        };
        timerfd_settime(&self.timer, TimerfdTimerFlags::empty(), &timer)?;
        Ok(())
    }

    fn silent_for(&self) -> Duration {
        self.alive_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .elapsed()
    }

    fn watch(&self) -> io::Result<()> {
        let mut expirations = [0; 8];
        loop {
            match rustix::io::read(&self.timer, &mut expirations) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
            let silent = self.silent_for();
            if silent < self.late_after {
                self.look_in(self.late_after - silent)?;
                continue;
            }
            if silent < LONGEST_DEVICE_WAIT && waits_for_a_device() {
                self.reassure();
            }
            self.look_in(LOOK)?;
        }
    }
}

impl Keepalive {
    pub fn start(socket: OsString, interval: Duration) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            socket,
            timer: timerfd_create(TimerfdClockId::Monotonic, TimerfdFlags::CLOEXEC)?,
            alive_at: Mutex::new(Instant::now()),
            late_after: interval + interval / 2,
        });
        let watched = Arc::clone(&shared);
        thread::Builder::new()
            .name("keepalive".to_owned())
            .spawn(move || {
                if let Err(error) = watched.watch() {
                    tracing::warn!("a busy disk may end the session: {error}");
                }
            })?;
        Ok(Self(shared))
    }

    pub fn alive(&self) {
        *self
            .0
            .alive_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Instant::now();
        self.0.reassure();
        if let Err(error) = self.0.look_in(self.0.late_after) {
            tracing::warn!("a busy disk may end the session: {error}");
        }
    }
}
