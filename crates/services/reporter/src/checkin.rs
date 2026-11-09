use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use serde_json::{Map, Value, json};

use crate::config::Config;
use crate::facts;
use crate::identity::Identity;
use crate::spool::Spool;

const STATE: &str = "checkin.json";
const NEW_SESSION: &str = "8d45620c1a4348dbb17410da57c60c66";
const SESSION_REMOVED: &str = "3354939424b4456d9802ca8333ed424a";
const SLEEP_START: &str = "6bbd95ee977941e497c48be27c254128";
const SLEEP_STOP: &str = "8811e6df2a8e40f58a94cea26f8ebf14";
const DESKTOP_SESSION: &str = "with class 'user' and type 'wayland'.";
const DAY_SECONDS: i64 = 24 * 60 * 60;
const MOST_DAYS: i64 = 7;
const SESSION_LOOKBACK_DAYS: i64 = 7;
const MICROS: i64 = 1_000_000;

#[derive(Default)]
struct State {
    last_day: Option<Date>,
    not_before: i64,
}

fn read_state(root: &Path) -> State {
    let Some(state) = fs::read(root.join(STATE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Map<String, Value>>(&bytes).ok())
    else {
        return State::default();
    };
    State {
        last_day: state
            .get("last_day")
            .and_then(Value::as_str)
            .and_then(|day| day.parse().ok()),
        not_before: state.get("not_before").and_then(Value::as_i64).unwrap_or(0),
    }
}

fn write_state(root: &Path, state: &State) -> io::Result<()> {
    let text = json!({
        "last_day": state.last_day.map(|day| day.to_string()),
        "not_before": state.not_before,
    });
    let staged = root.join(format!(".{STATE}.new"));
    fs::write(&staged, serde_json::to_vec(&text)?)?;
    fs::rename(staged, root.join(STATE))
}

pub fn yesterday() -> Option<Date> {
    Timestamp::now()
        .to_zoned(TimeZone::UTC)
        .date()
        .checked_sub(1.day())
        .ok()
}

pub fn due_days(root: &Path) -> Vec<Date> {
    let state = read_state(root);
    let Some(last) = yesterday() else {
        return Vec::new();
    };
    if Timestamp::now().as_second() < state.not_before {
        return Vec::new();
    }
    let earliest = last.checked_sub((MOST_DAYS - 1).days()).unwrap_or(last);
    let first = state
        .last_day
        .and_then(|day| day.tomorrow().ok())
        .map_or(last, |day| day.max(earliest));
    first
        .series(1.day())
        .take_while(|day| *day <= last)
        .collect()
}

fn in_window(root: &Path, due: usize) -> bool {
    let Some(identity) = Identity::load(root) else {
        return true;
    };
    let moment = Timestamp::now()
        .to_zoned(TimeZone::UTC)
        .date()
        .at(
            i8::try_from(identity.policy.checkin_hour).unwrap_or(0),
            i8::try_from(identity.checkin_minute).unwrap_or(0),
            0,
            0,
        )
        .to_zoned(TimeZone::UTC)
        .map(|moment| moment.timestamp());
    due > 1 || moment.is_ok_and(|moment| Timestamp::now() >= moment)
}

pub fn queue_if_due(spool: &Spool, config: &Config) {
    let due = due_days(spool.root()).len();
    if !config.sends_automatically() || due == 0 || !in_window(spool.root(), due) {
        return;
    }
    if let Err(error) = spool.queue_checkin() {
        tracing::warn!("cannot queue the daily check-in: {error}");
    }
}

pub fn sent(root: &Path, day: Date) -> io::Result<()> {
    let mut state = read_state(root);
    state.last_day = Some(day);
    write_state(root, &state)
}

pub fn forget(root: &Path) -> io::Result<()> {
    match fs::remove_file(root.join(STATE)) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

pub fn is_empty(payload: &Value) -> bool {
    ["boots", "sessions"]
        .iter()
        .all(|name| payload.get(name).and_then(Value::as_u64) == Some(0))
        && payload.get("active_hours").and_then(Value::as_f64) == Some(0.0)
}

pub fn not_yet_supported(root: &Path) -> io::Result<()> {
    let mut state = read_state(root);
    state.not_before = Timestamp::now().as_second() + DAY_SECONDS;
    write_state(root, &state)
}

fn journal(arguments: &[&str]) -> Vec<Map<String, Value>> {
    let output = Command::new("journalctl")
        .args(["--no-pager", "--quiet", "--all", "--output=json"])
        .args(arguments)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    output
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn text<'a>(entry: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    entry.get(name).and_then(Value::as_str)
}

fn seconds_of(entry: &Map<String, Value>) -> Option<i64> {
    let micros: i64 = text(entry, "__REALTIME_TIMESTAMP")?.parse().ok()?;
    Some(micros / MICROS)
}

struct Boot {
    id: String,
    first: i64,
    last: i64,
}

fn boots() -> Vec<Boot> {
    let output = Command::new("journalctl")
        .args(["--no-pager", "--list-boots", "--output=json"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| output.stdout)
        .unwrap_or_default();
    serde_json::from_slice::<Vec<Map<String, Value>>>(&output)
        .unwrap_or_default()
        .iter()
        .filter_map(|boot| {
            Some(Boot {
                id: text(boot, "boot_id")?.to_owned(),
                first: boot.get("first_entry")?.as_i64()? / MICROS,
                last: boot.get("last_entry")?.as_i64()? / MICROS,
            })
        })
        .collect()
}

fn union(mut spans: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    spans.sort_unstable();
    let mut merged: Vec<(i64, i64)> = Vec::new();
    for (start, end) in spans.into_iter().filter(|(start, end)| start < end) {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

fn overlap(spans: &[(i64, i64)], others: &[(i64, i64)]) -> i64 {
    spans
        .iter()
        .flat_map(|(start, end)| {
            others.iter().map(move |(other_start, other_end)| {
                (end.min(other_end) - start.max(other_start)).max(0)
            })
        })
        .sum()
}

fn spans_of(
    entries: &[Map<String, Value>],
    opens: &str,
    closes: &str,
    key: impl Fn(&Map<String, Value>) -> Option<String>,
    boots: &[Boot],
    now: i64,
) -> Vec<(i64, i64, String)> {
    let mut open: HashMap<String, (i64, String)> = HashMap::new();
    let mut spans = Vec::new();
    for entry in entries {
        let (Some(id), Some(time), Some(boot)) =
            (key(entry), seconds_of(entry), text(entry, "_BOOT_ID"))
        else {
            continue;
        };
        match text(entry, "MESSAGE_ID") {
            Some(message) if message == opens => {
                open.insert(id, (time, boot.to_owned()));
            }
            Some(message) if message == closes => {
                if let Some((start, _)) = open.remove(&id) {
                    spans.push((start, time, id));
                }
            }
            _ => {}
        }
    }
    for (id, (start, boot)) in open {
        let end = boots
            .iter()
            .find(|found| found.id == boot)
            .map_or(now, |found| found.last.min(now));
        spans.push((start, end, id));
    }
    spans
}

pub fn payload(day: Date) -> io::Result<Value> {
    let start = day
        .to_zoned(TimeZone::UTC)
        .map_err(io::Error::other)?
        .timestamp()
        .as_second();
    let end = start + DAY_SECONDS;
    let now = Timestamp::now().as_second();
    let boots = boots();
    let since = format!("--since=@{}", start - SESSION_LOOKBACK_DAYS * DAY_SECONDS);
    let until = format!("--until=@{end}");
    let session_events = journal(&[
        &since,
        &until,
        &format!("MESSAGE_ID={NEW_SESSION}"),
        "+",
        &format!("MESSAGE_ID={SESSION_REMOVED}"),
    ]);
    let person = |entry: &Map<String, Value>| {
        text(entry, "MESSAGE").is_some_and(|message| message.ends_with(DESKTOP_SESSION))
    };
    let people: Vec<Map<String, Value>> = session_events
        .into_iter()
        .filter(|entry| text(entry, "MESSAGE_ID") == Some(SESSION_REMOVED) || person(entry))
        .collect();
    let sessions = spans_of(
        &people,
        NEW_SESSION,
        SESSION_REMOVED,
        |entry| {
            Some(format!(
                "{}-{}",
                text(entry, "_BOOT_ID")?,
                text(entry, "SESSION_ID")?
            ))
        },
        &boots,
        now,
    );
    let started = sessions
        .iter()
        .filter(|(begin, ..)| (start..end).contains(begin))
        .count();
    let sleeps = journal(&[
        &since,
        &until,
        &format!("MESSAGE_ID={SLEEP_START}"),
        "+",
        &format!("MESSAGE_ID={SLEEP_STOP}"),
    ]);
    let asleep = spans_of(
        &sleeps,
        SLEEP_START,
        SLEEP_STOP,
        |entry| text(entry, "_BOOT_ID").map(str::to_owned),
        &boots,
        now,
    );
    let day_span = [(start, end)];
    let used = union(
        sessions
            .iter()
            .map(|(begin, finish, _)| (*begin, *finish))
            .collect(),
    );
    let used: Vec<(i64, i64)> = used
        .into_iter()
        .map(|(begin, finish)| (begin.max(start), finish.min(end)))
        .filter(|(begin, finish)| begin < finish)
        .collect();
    let asleep = union(
        asleep
            .iter()
            .map(|(begin, finish, _)| (*begin, *finish))
            .collect(),
    );
    let active = overlap(&used, &day_span) - overlap(&used, &asleep);
    let booted = boots
        .iter()
        .filter(|boot| (start..end).contains(&boot.first))
        .count();
    Ok(json!({
        "os_version": crate::spool::os_version(),
        "desktop_version": facts::installed_version(crate::spool::DESKTOP_PACKAGE),
        "arch": std::env::consts::ARCH,
        "active_hours": (active.max(0) as f64 / 360.0).round() / 10.0,
        "sessions": started,
        "boots": booted,
        "day": day.to_string(),
    }))
}
