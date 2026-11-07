use std::error::Error;
use std::path::Path;
use std::process::{Command, Stdio};

use jiff::Timestamp;
use serde_json::{Map, Value};

pub const COREDUMP_MESSAGE: &str = "fc2e22bc6ee647b6b90729ab34a250b1";
pub const PANIC_TARGET: &str = "panic";
const UNIT_FAILED_MESSAGE: &str = "d9b373ed55a64feb8242e02dbe79a49c";
const FIELDS: &str = "MESSAGE_ID,MESSAGE,TARGET,SYSLOG_IDENTIFIER,UNIT,USER_UNIT,UNIT_RESULT,_UID,_PID,COREDUMP_EXE,COREDUMP_COMM,COREDUMP_UID,COREDUMP_PID,COREDUMP_SIGNAL_NAME,COREDUMP_TIMESTAMP,COREDUMP_FILENAME,COREDUMP_CMDLINE,COREDUMP_PACKAGE_JSON,COREDUMP_USER_UNIT,COREDUMP_UNIT";

pub struct Entry(Map<String, Value>);

impl Entry {
    pub fn field(&self, name: &str) -> Option<String> {
        match self.0.get(name)? {
            Value::String(text) => Some(text.clone()),
            Value::Array(bytes) => {
                let bytes: Option<Vec<u8>> = bytes
                    .iter()
                    .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
                    .collect();
                Some(String::from_utf8_lossy(&bytes?).into_owned())
            }
            _ => None,
        }
    }

    pub fn number(&self, name: &str) -> Option<u32> {
        self.field(name)?.parse().ok()
    }

    pub fn timestamp(&self, name: &str) -> Option<Timestamp> {
        Timestamp::from_microsecond(self.field(name)?.parse().ok()?).ok()
    }

    pub fn time(&self) -> Timestamp {
        self.timestamp("__REALTIME_TIMESTAMP")
            .unwrap_or_else(Timestamp::now)
    }
}

fn journalctl(arguments: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = Command::new("journalctl")
        .args(["--no-pager", "--quiet"])
        .args(arguments)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn entries(text: &str) -> impl Iterator<Item = Entry> {
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok().map(Entry))
}

pub fn sync() {
    let synced = Command::new("journalctl")
        .arg("--sync")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if let Err(error) = synced {
        tracing::warn!("cannot sync the journal: {error}");
    }
}

pub fn new_failures(cursor: &Path) -> Result<Vec<Entry>, Box<dyn Error>> {
    let coredump = format!("MESSAGE_ID={COREDUMP_MESSAGE}");
    let failed = format!("MESSAGE_ID={UNIT_FAILED_MESSAGE}");
    let panic = format!("TARGET={PANIC_TARGET}");
    let text = journalctl(&[
        "--all",
        "--output=json",
        &format!("--output-fields={FIELDS}"),
        &format!("--cursor-file={}", cursor.display()),
        &coredump,
        "+",
        &failed,
        "+",
        &panic,
    ])?;
    Ok(entries(&text).collect())
}

fn until(time: Timestamp) -> String {
    format!("--until=@{}", time.as_second() + 1)
}

pub fn panic_of(pid: &str, time: Timestamp) -> Option<String> {
    let text = journalctl(&[
        "--all",
        "--output=json",
        "--output-fields=MESSAGE",
        "-n",
        "1",
        &until(time),
        &format!("_PID={pid}"),
        &format!("TARGET={PANIC_TARGET}"),
    ])
    .ok()?;
    entries(&text).next()?.field("MESSAGE")
}

pub fn previous_boot_ended_cleanly() -> bool {
    let last_line = journalctl(&["--boot=-1", "-n", "1"]).unwrap_or_default();
    last_line.trim().is_empty()
        || !journalctl(&["--boot=-1", "-n", "1", "UNIT=shutdown.target"])
            .unwrap_or_default()
            .trim()
            .is_empty()
}

pub fn previous_boot_end() -> Option<Timestamp> {
    let text = journalctl(&["--boot=-1", "-n", "1", "--output=json"]).ok()?;
    Some(entries(&text).next()?.time())
}
