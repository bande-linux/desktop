use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

const MOST_FAILURES: usize = 5;
const FAILURE_MEMORY: Duration = Duration::from_secs(600);
const MOST_HANDSHAKES: usize = 16;
const MOST_HANDSHAKES_PER_NETWORK: usize = 8;
const MOST_VIEWERS: usize = 4;
const IPV6_NETWORK_BITS: u32 = 64;
const FAILURE_DELAY: Duration = Duration::from_secs(1);
const FAILURES_BEFORE_BRAKING: usize = 10;
const LONGEST_BRAKE: Duration = Duration::from_secs(30);

fn network(peer: IpAddr) -> Option<u128> {
    match peer {
        IpAddr::V4(_) => None,
        IpAddr::V6(address) => Some(address.to_bits() >> (128 - IPV6_NETWORK_BITS)),
    }
}

/// How much each address may try, so one device cannot keep others out, and how fast everyone
/// together may guess passwords.
#[derive(Default)]
pub struct Limits {
    failures: Mutex<HashMap<IpAddr, Vec<Instant>>>,
    handshaking: Mutex<Vec<IpAddr>>,
    brake: Mutex<()>,
    viewers: AtomicUsize,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A handshake in progress, one per address at a time.
pub struct Handshake {
    limits: Arc<Limits>,
    peer: IpAddr,
}

impl Drop for Handshake {
    fn drop(&mut self) {
        let mut handshaking = lock(&self.limits.handshaking);
        if let Some(at) = handshaking.iter().position(|peer| *peer == self.peer) {
            handshaking.swap_remove(at);
        }
    }
}

/// A place for a viewer, taken before the person at the computer is asked.
pub struct Seat(Arc<Limits>);

impl Drop for Seat {
    fn drop(&mut self) {
        self.0.viewers.fetch_sub(1, Ordering::AcqRel);
    }
}

/// How long to wait before answering a password, given the failures of all addresses within
/// the last ten minutes: a second after a few, doubling after that, at most half a minute.
fn brake_for(failures: usize) -> Duration {
    match failures.checked_sub(FAILURES_BEFORE_BRAKING) {
        None => Duration::ZERO,
        Some(beyond) => FAILURE_DELAY
            .saturating_mul(1 << beyond.min(5))
            .min(LONGEST_BRAKE),
    }
}

impl Limits {
    fn recent_failures(&self) -> MutexGuard<'_, HashMap<IpAddr, Vec<Instant>>> {
        let mut failures = lock(&self.failures);
        let now = Instant::now();
        failures.retain(|_, times| {
            times.retain(|time| now.duration_since(*time) < FAILURE_MEMORY);
            !times.is_empty()
        });
        failures
    }

    pub fn blocked(&self, peer: IpAddr) -> bool {
        self.recent_failures()
            .get(&peer)
            .is_some_and(|times| times.len() >= MOST_FAILURES)
    }

    pub fn failed(&self, peer: IpAddr) {
        lock(&self.failures)
            .entry(peer)
            .or_default()
            .push(Instant::now());
        thread::sleep(FAILURE_DELAY);
    }

    /// Waits before a password is checked once many failed lately, one handshake at a time,
    /// so guessing from many addresses is slow while a real viewer only waits.
    pub fn slow_down(&self) {
        let failures: usize = self.recent_failures().values().map(Vec::len).sum();
        let delay = brake_for(failures);
        if delay.is_zero() {
            return;
        }
        let _turn = lock(&self.brake);
        thread::sleep(delay);
    }

    /// Starts a handshake from `peer`, unless one from that address is under way, too many are
    /// from its IPv6 network, or too many are in all.
    pub fn handshake(self: &Arc<Self>, peer: IpAddr) -> Result<Handshake, &'static str> {
        let mut handshaking = lock(&self.handshaking);
        let from_network = handshaking
            .iter()
            .filter(|other| network(**other).is_some() && network(**other) == network(peer))
            .count();
        let refusal = match () {
            () if handshaking.contains(&peer) => Some("This computer is already connecting."),
            () if handshaking.len() >= MOST_HANDSHAKES
                || from_network >= MOST_HANDSHAKES_PER_NETWORK =>
            {
                Some("Too many computers are connecting. Try again in a minute.")
            }
            () => None,
        };
        if let Some(refusal) = refusal {
            return Err(refusal);
        }
        handshaking.push(peer);
        Ok(Handshake {
            limits: Arc::clone(self),
            peer,
        })
    }

    /// A place for a viewer, while fewer than four have one.
    pub fn seat(self: &Arc<Self>) -> Option<Seat> {
        self.viewers
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |viewers| {
                (viewers < MOST_VIEWERS).then_some(viewers + 1)
            })
            .ok()
            .map(|_| Seat(Arc::clone(self)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_from_everywhere_slow_every_answer_down_without_shutting_anyone_out() {
        assert_eq!(brake_for(9), Duration::ZERO);
        assert_eq!(brake_for(10), Duration::from_secs(1));
        assert_eq!(brake_for(12), Duration::from_secs(4));
        assert_eq!(brake_for(1000), LONGEST_BRAKE);
        let limits = Arc::new(Limits::default());
        let one: IpAddr = "fd00::1".parse().unwrap();
        let other: IpAddr = "fd00::2".parse().unwrap();
        let first = limits.handshake(one).unwrap();
        assert!(limits.handshake(one).is_err(), "one handshake per address");
        assert!(
            limits.handshake(other).is_ok(),
            "a neighbour in the same /64 still gets in"
        );
        drop(first);
        assert!(limits.handshake(one).is_ok());
    }
}
