use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use rustix::io::{Errno, read};
use rustix::time::{
    ClockId, Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, clock_gettime,
    timerfd_create, timerfd_settime,
};

const ONCE: Timespec = Timespec {
    tv_sec: 0,
    tv_nsec: 0,
};

fn next_minute(now: Timespec) -> Timespec {
    Timespec {
        tv_sec: now.tv_sec.div_euclid(60) * 60 + 60,
        tv_nsec: 0,
    }
}

pub struct MinuteTimer {
    timer: OwnedFd,
}

impl MinuteTimer {
    pub fn new() -> rustix::io::Result<Self> {
        let timer = Self {
            timer: timerfd_create(TimerfdClockId::Realtime, TimerfdFlags::CLOEXEC)?,
        };
        timer.arm()?;
        Ok(timer)
    }

    fn arm(&self) -> rustix::io::Result<()> {
        timerfd_settime(
            &self.timer,
            TimerfdTimerFlags::ABSTIME | TimerfdTimerFlags::CANCEL_ON_SET,
            &Itimerspec {
                it_interval: ONCE,
                it_value: next_minute(clock_gettime(ClockId::Realtime)),
            },
        )
        .map(drop)
    }

    pub fn wait(&self) -> rustix::io::Result<()> {
        let mut expirations = [0; 8];
        loop {
            match read(&self.timer, &mut expirations) {
                Ok(_) | Err(Errno::CANCELED) => return self.arm(),
                Err(Errno::INTR) => continue,
                Err(error) => return Err(error),
            }
        }
    }
}

impl AsFd for MinuteTimer {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.timer.as_fd()
    }
}

const FAR_FUTURE: Timespec = Timespec {
    tv_sec: 4_294_967_295,
    tv_nsec: 0,
};

pub struct ClockChanges {
    timer: OwnedFd,
}

impl ClockChanges {
    pub fn new() -> rustix::io::Result<Self> {
        let changes = Self {
            timer: timerfd_create(
                TimerfdClockId::Realtime,
                TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
            )?,
        };
        changes.arm()?;
        Ok(changes)
    }

    fn arm(&self) -> rustix::io::Result<()> {
        timerfd_settime(
            &self.timer,
            TimerfdTimerFlags::ABSTIME | TimerfdTimerFlags::CANCEL_ON_SET,
            &Itimerspec {
                it_interval: ONCE,
                it_value: FAR_FUTURE,
            },
        )
        .map(drop)
    }

    pub fn take(&self) -> rustix::io::Result<bool> {
        let mut expirations = [0; 8];
        loop {
            match read(&self.timer, &mut expirations) {
                Ok(_) | Err(Errno::CANCELED) => return self.arm().map(|()| true),
                Err(Errno::AGAIN) => return Ok(false),
                Err(Errno::INTR) => continue,
                Err(error) => return Err(error),
            }
        }
    }
}

impl AsFd for ClockChanges {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.timer.as_fd()
    }
}
