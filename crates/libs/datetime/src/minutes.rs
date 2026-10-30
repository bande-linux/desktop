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

pub fn follow_minutes(mut minute: impl FnMut() -> bool) -> rustix::io::Result<()> {
    let timer = timerfd_create(TimerfdClockId::Realtime, TimerfdFlags::CLOEXEC)?;
    let mut expirations = [0; 8];
    loop {
        timerfd_settime(
            &timer,
            TimerfdTimerFlags::ABSTIME | TimerfdTimerFlags::CANCEL_ON_SET,
            &Itimerspec {
                it_interval: ONCE,
                it_value: next_minute(clock_gettime(ClockId::Realtime)),
            },
        )?;
        match read(&timer, &mut expirations) {
            Ok(_) | Err(Errno::CANCELED) => {}
            Err(Errno::INTR) => continue,
            Err(error) => return Err(error),
        }
        if !minute() {
            return Ok(());
        }
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
