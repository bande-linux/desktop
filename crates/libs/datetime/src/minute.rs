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

#[cfg(test)]
mod tests {
    use rustix::time::Timespec;

    use super::next_minute;

    fn at(tv_sec: i64, tv_nsec: i64) -> Timespec {
        Timespec { tv_sec, tv_nsec }
    }

    #[test]
    fn the_clock_wakes_on_the_next_whole_minute() {
        assert_eq!(next_minute(at(1_800_000_030, 500)).tv_sec, 1_800_000_060);
        assert_eq!(next_minute(at(1_800_000_000, 0)).tv_sec, 1_800_000_060);
        assert_eq!(
            next_minute(at(1_800_000_059, 999_999_999)).tv_sec,
            1_800_000_060
        );
    }
}
