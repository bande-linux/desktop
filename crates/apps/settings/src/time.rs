use std::sync::mpsc::{self, Sender};

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;

use unbound_settings_panes::SettingId;

use crate::failure::failed;
use crate::follow::{Update, Updates, spawn};

const TIMEDATE: &str = "org.freedesktop.timedate1";
const TIMEDATE_PATH: &str = "/org/freedesktop/timedate1";
pub const MANUAL_FORMAT: &str = "%Y-%m-%d %H:%M";

#[zbus::proxy(
    interface = "org.freedesktop.timedate1",
    default_service = "org.freedesktop.timedate1",
    default_path = "/org/freedesktop/timedate1"
)]
trait Timedate {
    #[zbus(property)]
    fn timezone(&self) -> zbus::Result<String>;
    #[zbus(property, name = "NTP")]
    fn ntp(&self) -> zbus::Result<bool>;
    #[zbus(property, name = "CanNTP")]
    fn can_ntp(&self) -> zbus::Result<bool>;
    fn list_timezones(&self) -> zbus::Result<Vec<String>>;
    fn set_timezone(&self, zone: &str, interactive: bool) -> zbus::Result<()>;
    #[zbus(name = "SetNTP")]
    fn set_ntp(&self, on: bool, interactive: bool) -> zbus::Result<()>;
    fn set_time(&self, microseconds: i64, relative: bool, interactive: bool) -> zbus::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TimeState {
    pub automatic: bool,
    pub can_automatic: bool,
    pub zone: String,
    pub zones: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TimeCommand {
    Automatic(bool),
    Zone(String),
    Set(Timestamp),
}

pub fn zone_place(zone: &str) -> (String, String) {
    let mut parts: Vec<String> = zone.split('/').map(|part| part.replace('_', " ")).collect();
    let city = parts.pop().unwrap_or_default();
    (city, parts.join(" › "))
}

pub fn zone_offset(zone: &str, at: Timestamp) -> Option<String> {
    let zone = TimeZone::get(zone).ok()?;
    let info = zone.to_offset_info(at);
    let seconds = info.offset().seconds();
    let sign = if seconds < 0 { '−' } else { '+' };
    let (hours, minutes) = (seconds.abs() / 3600, seconds.abs() % 3600 / 60);
    let offset = if minutes == 0 {
        format!("UTC{sign}{hours}")
    } else {
        format!("UTC{sign}{hours}:{minutes:02}")
    };
    let abbreviation = info.abbreviation();
    Some(
        if abbreviation.starts_with(['+', '-']) || abbreviation.is_empty() {
            offset
        } else {
            format!("{offset} ({abbreviation})")
        },
    )
}

pub fn zone_matches(zones: &[String], query: &str, limit: usize) -> Vec<String> {
    let query = query.trim().to_lowercase().replace(' ', "_");
    if query.is_empty() {
        return Vec::new();
    }
    let mut ranked: Vec<(u8, &String)> = zones
        .iter()
        .filter_map(|zone| {
            let lower = zone.to_lowercase();
            let city = lower.rsplit('/').next().unwrap_or_default();
            let rank = if city.starts_with(&query) {
                0
            } else if city.split('_').any(|word| word.starts_with(&query)) {
                1
            } else if lower.contains(&query) {
                2
            } else {
                return None;
            };
            Some((rank, zone))
        })
        .collect();
    ranked.sort();
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, zone)| zone.clone())
        .collect()
}

pub fn parse_manual(text: &str, zone: &str) -> Option<Timestamp> {
    let civil = DateTime::strptime(MANUAL_FORMAT, text.trim()).ok()?;
    let zone = TimeZone::get(zone).unwrap_or(TimeZone::UTC);
    Some(civil.to_zoned(zone).ok()?.timestamp())
}

fn read(timedate: &TimedateProxyBlocking<'_>, zones: &mut Vec<String>) -> Option<TimeState> {
    if zones.is_empty() {
        *zones = timedate.list_timezones().unwrap_or_default();
    }
    Some(TimeState {
        automatic: timedate.ntp().ok()?,
        can_automatic: timedate.can_ntp().unwrap_or(false),
        zone: timedate.timezone().ok()?,
        zones: zones.clone(),
    })
}

fn run(timedate: &TimedateProxyBlocking<'_>, command: TimeCommand) -> zbus::Result<()> {
    match command {
        TimeCommand::Automatic(on) => timedate.set_ntp(on, true),
        TimeCommand::Zone(zone) => timedate.set_timezone(&zone, true),
        TimeCommand::Set(at) => timedate.set_time(at.as_microsecond(), false, true),
    }
}

pub fn follow_time(system: &Connection, updates: Updates) -> Sender<TimeCommand> {
    let (sender, receiver) = mpsc::channel();
    let reader = system.clone();
    let feed = updates.clone();
    spawn("time", move || {
        let follow = || -> zbus::Result<()> {
            let timedate = TimedateProxyBlocking::builder(&reader)
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()?;
            let changes = PropertiesProxy::builder(&reader)
                .destination(TIMEDATE)?
                .path(TIMEDATE_PATH)?
                .build()?
                .receive_properties_changed()?;
            let mut zones = Vec::new();
            if !feed.send(Update::Time(read(&timedate, &mut zones))) {
                return Ok(());
            }
            for _ in changes {
                if !feed.send(Update::Time(read(&timedate, &mut zones))) {
                    break;
                }
            }
            Ok(())
        };
        if let Err(error) = follow() {
            tracing::warn!("cannot follow timedated: {error}");
        }
    });
    let system = system.clone();
    spawn("time-worker", move || {
        let Ok(timedate) = TimedateProxyBlocking::new(&system) else {
            return;
        };
        let mut zones = Vec::new();
        for command in receiver {
            let setting = match command {
                TimeCommand::Automatic(_) => SettingId::AutomaticTime,
                TimeCommand::Zone(_) => SettingId::TimeZone,
                TimeCommand::Set(_) => SettingId::DateAndTime,
            };
            if let Err(error) = run(&timedate, command) {
                updates.send(failed(setting, &error, "service-time"));
                updates.send(Update::Time(read(&timedate, &mut zones)));
            }
        }
    });
    sender
}
