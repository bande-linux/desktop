use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

const MOST_FAILURES: usize = 5;
const FAILURE_MEMORY: Duration = Duration::from_secs(600);
const MOST_HANDSHAKES: usize = 16;
const MOST_VIEWERS: usize = 4;
const IPV6_NETWORK_BITS: u32 = 64;

/// Where connections come from: an IPv4 address, or the /64 of an IPv6 one, since a device
/// picks addresses in its /64 freely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Source(u128);

impl From<IpAddr> for Source {
    fn from(peer: IpAddr) -> Self {
        match peer {
            IpAddr::V4(address) => Self(u128::from(address.to_bits())),
            IpAddr::V6(address) => Self(address.to_bits() >> (128 - IPV6_NETWORK_BITS) | 1 << 127),
        }
    }
}

/// How much each source may try, so one device cannot keep others out.
#[derive(Default)]
pub struct Limits {
    failures: Mutex<HashMap<Source, Vec<Instant>>>,
    handshaking: Mutex<HashSet<Source>>,
    viewers: AtomicUsize,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A handshake in progress, one per source at a time.
pub struct Handshake {
    limits: Arc<Limits>,
    source: Source,
}

impl Drop for Handshake {
    fn drop(&mut self) {
        lock(&self.limits.handshaking).remove(&self.source);
    }
}

/// A viewer let in.
pub struct Seat(Arc<Limits>);

impl Drop for Seat {
    fn drop(&mut self) {
        self.0.viewers.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Limits {
    pub fn blocked(&self, peer: IpAddr) -> bool {
        let mut failures = lock(&self.failures);
        let now = Instant::now();
        failures.retain(|_, times| {
            times.retain(|time| now.duration_since(*time) < FAILURE_MEMORY);
            !times.is_empty()
        });
        failures
            .get(&Source::from(peer))
            .is_some_and(|times| times.len() >= MOST_FAILURES)
    }

    pub fn failed(&self, peer: IpAddr) {
        lock(&self.failures)
            .entry(Source::from(peer))
            .or_default()
            .push(Instant::now());
    }

    /// Starts a handshake from `peer`, unless one from there is under way or too many are.
    pub fn handshake(self: &Arc<Self>, peer: IpAddr) -> Option<Handshake> {
        let source = Source::from(peer);
        let mut handshaking = lock(&self.handshaking);
        if handshaking.len() >= MOST_HANDSHAKES || !handshaking.insert(source) {
            return None;
        }
        Some(Handshake {
            limits: Arc::clone(self),
            source,
        })
    }

    /// A seat for a viewer that was let in, while fewer than four are.
    pub fn seat(self: &Arc<Self>) -> Option<Seat> {
        self.viewers
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |viewers| {
                (viewers < MOST_VIEWERS).then_some(viewers + 1)
            })
            .ok()
            .map(|_| Seat(Arc::clone(self)))
    }
}
