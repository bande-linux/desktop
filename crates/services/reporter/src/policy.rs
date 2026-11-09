use std::collections::HashMap;
use std::time::Duration;

use serde_json::{Value, json};

const MIN_INTERVAL: Duration = Duration::from_secs(15 * 60);
const SHORTEST_INTERVAL: Duration = Duration::from_secs(1);
const LONGEST_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const DAILY_REPORTS: u64 = 50;
const DAILY_BYTES: u64 = 20 * 1024 * 1024;
const CHECKIN_HOUR: u8 = 13;
const ALWAYS_SENT: &str = "manual";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub min_interval: Duration,
    pub daily_reports: u64,
    pub daily_bytes: u64,
    pub kinds_off: Vec<String>,
    pub counts_only: Vec<String>,
    pub sample: HashMap<String, u64>,
    pub checkin_hour: u8,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            min_interval: MIN_INTERVAL,
            daily_reports: DAILY_REPORTS,
            daily_bytes: DAILY_BYTES,
            kinds_off: Vec::new(),
            counts_only: Vec::new(),
            sample: HashMap::new(),
            checkin_hour: CHECKIN_HOUR,
        }
    }
}

fn texts(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub enum Choice {
    Send,
    CountsOnly,
    KeepHere,
}

impl Policy {
    pub fn from_json(value: &Value) -> Self {
        let defaults = Self::default();
        let number = |name| value.get(name).and_then(Value::as_u64);
        Self {
            min_interval: number("min_interval")
                .map(Duration::from_secs)
                .map_or(defaults.min_interval, |interval| {
                    interval.clamp(SHORTEST_INTERVAL, LONGEST_INTERVAL)
                }),
            daily_reports: number("daily_reports").unwrap_or(defaults.daily_reports),
            daily_bytes: number("daily_bytes").unwrap_or(defaults.daily_bytes),
            kinds_off: texts(value.get("kinds_off")),
            counts_only: texts(value.get("counts_only")),
            sample: value
                .get("sample")
                .and_then(Value::as_object)
                .map(|sample| {
                    sample
                        .iter()
                        .filter_map(|(kind, every)| Some((kind.clone(), every.as_u64()?)))
                        .filter(|(_, every)| *every >= 2)
                        .collect()
                })
                .unwrap_or_default(),
            checkin_hour: number("checkin_hour")
                .and_then(|hour| u8::try_from(hour).ok())
                .filter(|hour| *hour < 24)
                .unwrap_or(defaults.checkin_hour),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "min_interval": self.min_interval.as_secs(),
            "daily_reports": self.daily_reports,
            "daily_bytes": self.daily_bytes,
            "kinds_off": self.kinds_off,
            "counts_only": self.counts_only,
            "sample": self.sample,
            "checkin_hour": self.checkin_hour,
        })
    }

    pub fn choose(&self, kind: &str, signature: &str, id: &str) -> Choice {
        if kind == ALWAYS_SENT {
            return Choice::Send;
        }
        if self.kinds_off.iter().any(|off| off == kind) {
            return Choice::KeepHere;
        }
        if let Some(every) = self.sample.get(kind)
            && !chosen(id, *every)
        {
            return Choice::KeepHere;
        }
        if self.counts_only.iter().any(|only| only == signature) {
            return Choice::CountsOnly;
        }
        Choice::Send
    }
}

fn chosen(id: &str, every: u64) -> bool {
    let digest = ring::digest::digest(&ring::digest::SHA256, id.as_bytes());
    let mut first = [0; 8];
    first.copy_from_slice(&digest.as_ref()[..8]);
    u64::from_le_bytes(first) % every == 0
}
