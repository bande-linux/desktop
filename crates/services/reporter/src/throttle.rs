use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Map, Value, json};

use crate::policy::Policy;

const FILE: &str = "sending.json";
const EXEMPT_KINDS: [&str; 3] = ["kernel-panic", "failed-boot", "manual"];
const EXEMPT_DAILY_REPORTS: u64 = 10;
const JITTER_PERCENT: u64 = 20;
const NEXT_DAY_SPREAD: u64 = 60 * 60;
const NETWORK_MANAGER: [&str; 4] = [
    "org.freedesktop.NetworkManager",
    "/org/freedesktop/NetworkManager",
    "org.freedesktop.NetworkManager",
    "Metered",
];
const METERED_YES: u32 = 1;
const METERED_GUESS_YES: u32 = 3;
pub const LARGE_ATTACHMENTS: u64 = 1024 * 1024;

#[derive(Default)]
pub struct Ledger {
    day: Option<Date>,
    reports: u64,
    bytes: u64,
    exempt: u64,
    last_run_ms: i64,
}

pub fn today() -> Date {
    Timestamp::now().to_zoned(TimeZone::UTC).date()
}

pub fn is_exempt(kind: &str) -> bool {
    EXEMPT_KINDS.contains(&kind)
}

impl Ledger {
    pub fn load(root: &Path) -> Self {
        let Some(state) = fs::read(root.join(FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Map<String, Value>>(&bytes).ok())
        else {
            return Self::default();
        };
        let number = |name| state.get(name).and_then(Value::as_u64).unwrap_or(0);
        let mut ledger = Self {
            day: state
                .get("day")
                .and_then(Value::as_str)
                .and_then(|day| day.parse().ok()),
            reports: number("reports"),
            bytes: number("bytes"),
            exempt: number("exempt"),
            last_run_ms: state
                .get("last_run_ms")
                .and_then(Value::as_i64)
                .unwrap_or(0),
        };
        ledger.roll();
        ledger
    }

    pub fn save(&self, root: &Path) -> io::Result<()> {
        let state = json!({
            "day": self.day.map(|day| day.to_string()),
            "reports": self.reports,
            "bytes": self.bytes,
            "exempt": self.exempt,
            "last_run_ms": self.last_run_ms,
        });
        let staged = root.join(format!(".{FILE}.new"));
        fs::write(&staged, serde_json::to_vec(&state)?)?;
        fs::rename(staged, root.join(FILE))
    }

    fn roll(&mut self) {
        let today = today();
        if self.day != Some(today) {
            *self = Self {
                day: Some(today),
                last_run_ms: self.last_run_ms,
                ..Self::default()
            };
        }
    }

    pub fn allows(&mut self, policy: &Policy, kind: &str, bytes: u64) -> bool {
        self.roll();
        if is_exempt(kind) {
            return self.exempt < EXEMPT_DAILY_REPORTS;
        }
        self.reports < policy.daily_reports
            && self.bytes.saturating_add(bytes) <= policy.daily_bytes
    }

    pub fn spend(&mut self, kind: &str, bytes: u64) {
        self.roll();
        if is_exempt(kind) {
            self.exempt += 1;
        } else {
            self.reports += 1;
            self.bytes = self.bytes.saturating_add(bytes);
        }
    }

    pub fn next_run_in(&self, policy: &Policy) -> Duration {
        let interval = i64::try_from(policy.min_interval.as_millis()).unwrap_or(i64::MAX);
        let next = self.last_run_ms.saturating_add(interval);
        Duration::from_millis(
            next.saturating_sub(Timestamp::now().as_millisecond())
                .max(0)
                .unsigned_abs(),
        )
    }

    pub fn ran(&mut self) {
        self.last_run_ms = Timestamp::now().as_millisecond();
    }
}

pub fn random_below(bound: u64) -> u64 {
    let mut bytes = [0; 8];
    if SystemRandom::new().fill(&mut bytes).is_err() || bound == 0 {
        return 0;
    }
    u64::from_le_bytes(bytes) % bound
}

pub fn jittered(wait: Duration) -> Duration {
    let millis = u64::try_from(wait.as_millis()).unwrap_or(u64::MAX);
    let spread = millis / 100 * JITTER_PERCENT;
    Duration::from_millis(
        millis
            .saturating_sub(spread)
            .saturating_add(random_below(spread.saturating_mul(2).saturating_add(1))),
    )
}

pub fn until_next_day() -> Duration {
    let now = Timestamp::now();
    let next = today()
        .tomorrow()
        .ok()
        .and_then(|day| day.to_zoned(TimeZone::UTC).ok())
        .map_or(now.as_second(), |start| start.timestamp().as_second());
    Duration::from_secs(
        next.saturating_sub(now.as_second()).max(0).unsigned_abs() + random_below(NEXT_DAY_SPREAD),
    )
}

pub fn is_metered() -> bool {
    let output = Command::new("busctl")
        .args(["--system", "get-property"])
        .args(NETWORK_MANAGER)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let Ok(output) = output else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u32>().ok())
        .is_some_and(|value| value == METERED_YES || value == METERED_GUESS_YES)
}

pub fn on_mains() -> bool {
    Command::new("systemd-ac-power")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.code() != Some(1))
}
