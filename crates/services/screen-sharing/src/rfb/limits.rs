use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const MOST_FAILURES: usize = 5;
const MOST_FAILURES_OVERALL: usize = 30;
const FAILURE_MEMORY: Duration = Duration::from_secs(600);

#[derive(Default)]
pub struct Attempts {
    failures: Mutex<HashMap<IpAddr, Vec<Instant>>>,
}

impl Attempts {
    fn recent(&self, peer: IpAddr) -> (usize, usize) {
        let mut failures = self
            .failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        failures.retain(|_, times| {
            times.retain(|time| now.duration_since(*time) < FAILURE_MEMORY);
            !times.is_empty()
        });
        let overall = failures.values().map(Vec::len).sum();
        (failures.get(&peer).map_or(0, Vec::len), overall)
    }

    pub fn blocked(&self, peer: IpAddr) -> bool {
        let (from_peer, overall) = self.recent(peer);
        from_peer >= MOST_FAILURES || overall >= MOST_FAILURES_OVERALL
    }

    pub fn failed(&self, peer: IpAddr) {
        self.failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(peer)
            .or_default()
            .push(Instant::now());
    }
}
