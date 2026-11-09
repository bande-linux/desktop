use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use jiff::Timestamp;
use serde_json::{Map, Value, json};

use crate::policy::Policy;
use crate::throttle::random_below;

const FILE: &str = "identity";
const MINUTES: u64 = 60;
const HARDEST_PROOF: u32 = 30;
const LONGEST_NONCE: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub machine: String,
    pub token: String,
    pub rotate_after: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub endpoint: String,
    pub credentials: Option<Credentials>,
    pub rotate: bool,
    pub revoked: bool,
    pub checkin_minute: u8,
    pub policy: Policy,
}

impl Identity {
    pub fn new(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.to_owned(),
            credentials: None,
            rotate: false,
            revoked: false,
            checkin_minute: u8::try_from(random_below(MINUTES)).unwrap_or(0),
            policy: Policy::default(),
        }
    }

    pub fn load(root: &Path) -> Option<Self> {
        let state: Map<String, Value> =
            serde_json::from_slice(&fs::read(root.join(FILE)).ok()?).ok()?;
        let text = |name| state.get(name).and_then(Value::as_str).map(str::to_owned);
        let credentials = match (text("machine"), text("token")) {
            (Some(machine), Some(token)) => Some(Credentials {
                machine,
                token,
                rotate_after: text("rotate_after").and_then(|time| time.parse().ok()),
            }),
            _ => None,
        };
        Some(Self {
            endpoint: text("endpoint")?,
            credentials,
            rotate: state
                .get("rotate")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            revoked: state
                .get("revoked")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            checkin_minute: state
                .get("checkin_minute")
                .and_then(Value::as_u64)
                .and_then(|minute| u8::try_from(minute).ok())
                .filter(|minute| u64::from(*minute) < MINUTES)
                .unwrap_or(0),
            policy: state
                .get("policy")
                .map(Policy::from_json)
                .unwrap_or_default(),
        })
    }

    pub fn save(&self, root: &Path) -> io::Result<()> {
        let credentials = self.credentials.as_ref();
        let state = json!({
            "endpoint": self.endpoint,
            "machine": credentials.map(|credentials| &credentials.machine),
            "token": credentials.map(|credentials| &credentials.token),
            "rotate_after": credentials
                .and_then(|credentials| credentials.rotate_after)
                .map(|time| time.to_string()),
            "rotate": self.rotate,
            "revoked": self.revoked,
            "checkin_minute": self.checkin_minute,
            "policy": self.policy.to_json(),
        });
        let staged = root.join(format!(".{FILE}.new"));
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&staged)?;
        file.write_all(&serde_json::to_vec(&state)?)?;
        file.sync_all()?;
        fs::rename(&staged, root.join(FILE))?;
        fs::File::open(root)?.sync_all()
    }

    pub fn learn(&mut self, answer: &Value) {
        if let Some(policy) = answer.get("policy").filter(|policy| policy.is_object()) {
            self.policy = Policy::from_json(policy);
        }
        if answer.get("rotate").and_then(Value::as_bool) == Some(true) {
            self.rotate = true;
        }
    }

    pub fn take_credentials(&mut self, answer: &Value) -> bool {
        let text = |name| answer.get(name).and_then(Value::as_str).map(str::to_owned);
        let (Some(machine), Some(token)) = (text("machine"), text("token")) else {
            return false;
        };
        self.credentials = Some(Credentials {
            machine,
            token,
            rotate_after: text("rotate_after").and_then(|time| time.parse().ok()),
        });
        self.rotate = false;
        self.revoked = false;
        self.learn(answer);
        true
    }

    pub fn rotation_due(&self) -> bool {
        self.rotate
            || self
                .credentials
                .as_ref()
                .and_then(|credentials| credentials.rotate_after)
                .is_some_and(|after| Timestamp::now() >= after)
    }
}

pub fn forget(root: &Path) -> io::Result<()> {
    match fs::remove_file(root.join(FILE)) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn leading_zero_bits(digest: &[u8]) -> u32 {
    let mut bits = 0;
    for byte in digest {
        bits += byte.leading_zeros();
        if *byte != 0 {
            break;
        }
    }
    bits
}

pub fn solve(challenge: &str, difficulty: u32) -> Option<String> {
    if difficulty > HARDEST_PROOF {
        return None;
    }
    (0..u64::MAX)
        .map(|counter| counter.to_string())
        .take_while(|nonce| nonce.len() <= LONGEST_NONCE)
        .find(|nonce| {
            let digest = ring::digest::digest(
                &ring::digest::SHA256,
                format!("{challenge}:{nonce}").as_bytes(),
            );
            leading_zero_bits(digest.as_ref()) >= difficulty
        })
}
