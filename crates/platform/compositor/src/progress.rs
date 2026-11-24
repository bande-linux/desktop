use std::time::Duration;

/// How long a program that should show itself may do nothing at all before it counts as hung:
/// longer than D-Bus lets a service take to start, which it may be waiting for.
pub const HUNG_AFTER: Duration = Duration::from_secs(30);

/// Whether a program that has shown nothing yet is slow or hung, told by the processor time and
/// the page reads the kernel counts for it.
pub struct Progress {
    pid: u32,
    seen: Option<(u64, u64)>,
    since: Duration,
}

struct Work {
    waits_for_a_device: bool,
    done: (u64, u64),
}

fn work_in(stat: &str) -> Option<Work> {
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let field = |index: usize| fields.get(index)?.parse::<u64>().ok();
    Some(Work {
        waits_for_a_device: *fields.first()? == "D",
        done: (field(9)?, field(11)? + field(12)?),
    })
}

impl Progress {
    pub fn of(pid: u32, now: Duration) -> Self {
        Self {
            pid,
            seen: None,
            since: now,
        }
    }

    /// Looks at the program, telling whether it has done nothing for [`HUNG_AFTER`].
    pub fn hung(&mut self, now: Duration) -> bool {
        let work = std::fs::read_to_string(format!("/proc/{}/stat", self.pid))
            .inspect_err(|error| tracing::debug!("cannot look at process {}: {error}", self.pid))
            .ok()
            .and_then(|stat| work_in(&stat));
        if let Some(work) = work
            && (work.waits_for_a_device || self.seen != Some(work.done))
        {
            self.seen = Some(work.done);
            self.since = now;
        }
        now.saturating_sub(self.since) >= HUNG_AFTER
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn after(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    #[test]
    fn only_a_program_that_does_nothing_for_long_counts_as_hung() -> std::io::Result<()> {
        let mut idle = Command::new("sleep").arg("60").spawn()?;
        std::thread::sleep(Duration::from_millis(200));
        let mut progress = Progress::of(idle.id(), after(0));
        assert!(!progress.hung(after(1)));
        assert!(!progress.hung(after(30)));
        assert!(progress.hung(after(31)));
        idle.kill()?;
        idle.wait()?;

        let mut busy = Command::new("sh")
            .args(["-c", "while :; do :; done"])
            .stdout(Stdio::null())
            .spawn()?;
        let mut progress = Progress::of(busy.id(), after(0));
        for minute in 1..4 {
            std::thread::sleep(Duration::from_millis(100));
            assert!(!progress.hung(after(minute * 60)), "{minute}");
        }
        busy.kill()?;
        busy.wait()?;
        Ok(())
    }

    #[test]
    fn a_program_waiting_for_a_device_is_not_hung() {
        let stat = |state| format!("7 (a b) c) {state} 1 1 1 0 -1 0 5 0 9 0 20 30 0 0 20 0 1 0 1");
        let work = work_in(&stat("D"));
        assert!(work.is_some_and(|work| work.waits_for_a_device && work.done == (9, 50)));
        assert!(work_in(&stat("S")).is_some_and(|work| !work.waits_for_a_device));
        assert!(work_in("7 (x").is_none());
    }
}
