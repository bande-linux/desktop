use std::io;
use std::os::fd::OwnedFd;
use std::time::{Duration, Instant};

use rustix::event::{EventfdFlags, PollFd, PollFlags, eventfd, poll};
use rustix::io::{Errno, read, write};
use rustix::time::{
    ClockId, Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, clock_gettime,
    timerfd_create, timerfd_settime,
};

const DISARMED: Timespec = Timespec {
    tv_sec: 0,
    tv_nsec: 0,
};

fn elapsed(clock: ClockId) -> Duration {
    let time = clock_gettime(clock);
    Duration::new(
        u64::try_from(time.tv_sec).unwrap_or(0),
        u32::try_from(time.tv_nsec).unwrap_or(0),
    )
}

pub(crate) fn now() -> Instant {
    Instant::now() + elapsed(ClockId::Boottime).saturating_sub(elapsed(ClockId::Monotonic))
}

fn timespec(duration: Duration) -> Timespec {
    Timespec {
        tv_sec: i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
        tv_nsec: i64::from(duration.subsec_nanos()),
    }
}

pub(crate) struct Alarm {
    timer: OwnedFd,
    nudges: OwnedFd,
}

impl Alarm {
    pub(crate) fn new() -> io::Result<Self> {
        Ok(Self {
            timer: timerfd_create(
                TimerfdClockId::Boottime,
                TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
            )?,
            nudges: eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?,
        })
    }

    pub(crate) fn nudge(&self) {
        let _ = write(&self.nudges, &1u64.to_ne_bytes());
    }

    pub(crate) fn wait_until(&self, deadline: Option<Instant>) -> io::Result<()> {
        let value = deadline.map_or(DISARMED, |deadline| {
            timespec(
                deadline
                    .saturating_duration_since(now())
                    .max(Duration::from_nanos(1)),
            )
        });
        timerfd_settime(
            &self.timer,
            TimerfdTimerFlags::empty(),
            &Itimerspec {
                it_interval: DISARMED,
                it_value: value,
            },
        )?;
        let mut fds = [
            PollFd::new(&self.timer, PollFlags::IN),
            PollFd::new(&self.nudges, PollFlags::IN),
        ];
        match poll(&mut fds, None) {
            Ok(_) | Err(Errno::INTR) => {}
            Err(error) => return Err(error.into()),
        }
        let mut count = [0; 8];
        for fd in [&self.timer, &self.nudges] {
            match read(fd, &mut count) {
                Ok(_) | Err(Errno::AGAIN) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}
