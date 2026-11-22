use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use des::Des;
use des::cipher::{Block, BlockCipherEncrypt, KeyInit};

pub const CHALLENGE: usize = 16;
pub const PASSWORD_BYTES: usize = 8;
const MOST_FAILURES: usize = 5;
const FAILURE_MEMORY: Duration = Duration::from_secs(600);

fn reversed_key(password: &str) -> [u8; PASSWORD_BYTES] {
    let mut key = [0; PASSWORD_BYTES];
    for (slot, byte) in key.iter_mut().zip(password.bytes()) {
        *slot = byte.reverse_bits();
    }
    key
}

pub fn vnc_response(password: &str, challenge: &[u8; CHALLENGE]) -> [u8; CHALLENGE] {
    let cipher = Des::new(&reversed_key(password).into());
    let mut response = *challenge;
    for half in response.as_chunks_mut::<8>().0 {
        let mut block = Block::<Des>::from(*half);
        cipher.encrypt_block(&mut block);
        half.copy_from_slice(&block);
    }
    response
}

pub fn same(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

#[derive(Default)]
pub struct Attempts {
    failures: Mutex<HashMap<IpAddr, Vec<Instant>>>,
}

impl Attempts {
    fn recent(&self, peer: IpAddr) -> usize {
        let mut failures = self
            .failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        failures.retain(|_, times| {
            times.retain(|time| now.duration_since(*time) < FAILURE_MEMORY);
            !times.is_empty()
        });
        failures.get(&peer).map_or(0, Vec::len)
    }

    pub fn blocked(&self, peer: IpAddr) -> bool {
        self.recent(peer) >= MOST_FAILURES
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vnc_response_matches_the_reference() {
        let challenge = *b"0123456789abcdef";
        assert_eq!(
            vnc_response("password", &challenge),
            [
                0x56, 0x45, 0xab, 0xeb, 0x5f, 0x1e, 0x64, 0x75, 0xe8, 0xfe, 0xb1, 0x1b, 0xeb, 0x66,
                0xea, 0x19
            ]
        );
    }
}
