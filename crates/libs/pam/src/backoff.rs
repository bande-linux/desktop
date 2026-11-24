use std::time::{Duration, Instant};

const FREE_ATTEMPTS: u32 = 4;
const FIRST_WAIT: Duration = Duration::from_secs(5);
const LONGEST_WAIT: Duration = Duration::from_secs(60);

/// How long someone who keeps answering wrongly waits before the next answer: a few attempts are
/// free, then the wait doubles up to a minute.
#[derive(Debug, Default, Clone)]
pub struct Backoff {
    failures: u32,
    until: Option<Instant>,
}

impl Backoff {
    pub fn fail(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        let over = self.failures.saturating_sub(FREE_ATTEMPTS);
        if over > 0 {
            let wait = FIRST_WAIT
                .saturating_mul(1 << (over - 1).min(8))
                .min(LONGEST_WAIT);
            self.until = Some(now + wait);
        }
    }

    pub fn succeed(&mut self) {
        *self = Self::default();
    }

    /// When the wait ends, also once it has.
    pub fn until(&self) -> Option<Instant> {
        self.until
    }

    /// How much of the wait is left.
    pub fn waiting(&self, now: Instant) -> Option<Duration> {
        self.until
            .map(|until| until.saturating_duration_since(now))
            .filter(|left| !left.is_zero())
    }
}
