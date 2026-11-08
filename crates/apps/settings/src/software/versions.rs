use std::fs;
use std::path::Path;

use bande_packages::Error;
use zbus::MatchRule;
use zbus::blocking::{Connection, MessageIterator};
use zbus::proxy::MethodFlags;
use zbus::zvariant::OwnedObjectPath;

const VERSIONS: &str = "/run/bande/versions";
const WENT_BACK: &str = "/run/bande/went-back";
const PREPARE_UNIT: &str = "bande-system-prepare.service";
const SYSTEMD: &str = "org.freedesktop.systemd1";
const MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER: &str = "org.freedesktop.systemd1.Manager";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    Good,
    Pending,
    Trying,
    Failed,
    Kept,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub id: u32,
    pub created: i64,
    pub condition: Condition,
    pub running: bool,
    pub changed: usize,
    pub names: String,
}

fn parse_version(line: &str) -> Option<Version> {
    let mut fields = line.split('\t');
    let id = fields.next()?.parse().ok()?;
    let created = fields.next()?.parse().ok()?;
    fields.next()?;
    let condition = match fields.next()? {
        "good" => Condition::Good,
        "pending" => Condition::Pending,
        "trying" => Condition::Trying,
        "failed" => Condition::Failed,
        _ => Condition::Kept,
    };
    let running = fields.next()? == "running";
    let changed = fields.next()?.parse().ok()?;
    let names = fields.next().unwrap_or_default().replace(',', ", ");
    Some(Version {
        id,
        created,
        condition,
        running,
        changed,
        names,
    })
}

pub fn available() -> bool {
    Path::new(VERSIONS).exists()
}

pub fn read() -> Vec<Version> {
    fs::read_to_string(VERSIONS)
        .map(|text| text.lines().filter_map(parse_version).collect())
        .unwrap_or_default()
}

pub fn prepared() -> Option<usize> {
    read()
        .into_iter()
        .find(|version| version.condition == Condition::Pending)
        .map(|version| version.changed)
}

pub fn went_back() -> Option<i64> {
    let text = fs::read_to_string(WENT_BACK).ok()?;
    text.lines().last()?.split('\t').nth(1)?.parse().ok()
}

pub(crate) fn run_unit(system: &Connection, unit: &str) -> Result<(), Error> {
    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(SYSTEMD)?
        .interface(MANAGER)?
        .member("JobRemoved")?
        .build();
    let mut removed = MessageIterator::for_match_rule(rule, system, None)?;
    let manager = zbus::blocking::Proxy::new(system, SYSTEMD, MANAGER_PATH, MANAGER)?;
    manager.call_method("Subscribe", &())?;
    let job: OwnedObjectPath = manager
        .call_with_flags(
            "StartUnit",
            MethodFlags::AllowInteractiveAuth.into(),
            &(unit, "replace"),
        )?
        .ok_or(zbus::Error::InvalidReply)?;
    let finished = removed.find_map(|message| {
        let message = message.ok()?;
        let (_, path, _, result): (u32, OwnedObjectPath, String, String) =
            message.body().deserialize().ok()?;
        (path == job).then_some(result)
    });
    match finished.as_deref() {
        Some("done") => Ok(()),
        Some(result) => Err(zbus::Error::Failure(format!("{unit}: {result}")).into()),
        None => Err(zbus::Error::InvalidReply.into()),
    }
}

pub fn prepare(system: &Connection) -> Result<(), Error> {
    run_unit(system, PREPARE_UNIT)
}

pub fn restore(system: &Connection, id: u32) -> Result<(), Error> {
    run_unit(system, &format!("bande-system-restore@{id}.service"))
}
