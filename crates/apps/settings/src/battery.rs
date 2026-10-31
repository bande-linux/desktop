use std::sync::mpsc::{self, Sender};

use bande_status::history::charge_history;
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::OwnedValue;
use zbus::{MatchRule, message};

use crate::failure::unexpected;
use crate::follow::{Update, Updates, spawn};

const UPOWER_PATH: &str = "/org/freedesktop/UPower";
const CHARGING: u32 = 1;
const FULL: u32 = 4;
const PENDING_CHARGE: u32 = 5;
const HISTORY_SECONDS: u32 = 24 * 3600;
const HISTORY_POINTS: u32 = 96;

#[zbus::proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower/devices/DisplayDevice"
)]
trait Device {
    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn time_to_empty(&self) -> zbus::Result<i64>;
    #[zbus(property)]
    fn time_to_full(&self) -> zbus::Result<i64>;
}

#[zbus::proxy(
    interface = "org.freedesktop.UPower.PowerProfiles",
    default_service = "org.freedesktop.UPower.PowerProfiles",
    default_path = "/org/freedesktop/UPower/PowerProfiles"
)]
trait PowerProfiles {
    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_active_profile(&self, profile: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn profiles(&self) -> zbus::Result<Vec<std::collections::HashMap<String, OwnedValue>>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charge {
    Discharging { minutes: Option<u32> },
    Charging { minutes: Option<u32> },
    Full,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BatteryState {
    pub percent: f64,
    pub charge: Charge,
    pub history: Vec<(u32, f64)>,
    pub profiles: Vec<String>,
    pub profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatteryCommand {
    Profile(String),
}

pub fn charge_of(state: u32, to_empty: i64, to_full: i64) -> Charge {
    let minutes = |seconds: i64| {
        u32::try_from(seconds / 60)
            .ok()
            .filter(|minutes| *minutes > 0)
    };
    match state {
        FULL => Charge::Full,
        CHARGING | PENDING_CHARGE => Charge::Charging {
            minutes: minutes(to_full),
        },
        _ => Charge::Discharging {
            minutes: minutes(to_empty),
        },
    }
}

fn profile_names(profiles: &PowerProfilesProxyBlocking<'_>) -> Vec<String> {
    profiles
        .profiles()
        .unwrap_or_default()
        .iter()
        .filter_map(|profile| String::try_from(profile.get("Profile")?.try_clone().ok()?).ok())
        .collect()
}

fn read(system: &Connection) -> Option<BatteryState> {
    let display = DeviceProxyBlocking::builder(system)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .ok()?;
    if !display.is_present().ok()? {
        return None;
    }
    let history = charge_history(system, HISTORY_SECONDS, HISTORY_POINTS)
        .unwrap_or_default()
        .into_iter()
        .map(|sample| (sample.at, sample.percent))
        .collect();
    let profiles = PowerProfilesProxyBlocking::builder(system)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .ok();
    let profile = profiles
        .as_ref()
        .and_then(|profiles| profiles.active_profile().ok());
    Some(BatteryState {
        percent: display.percentage().ok()?,
        charge: charge_of(
            display.state().unwrap_or(0),
            display.time_to_empty().unwrap_or(0),
            display.time_to_full().unwrap_or(0),
        ),
        history,
        profiles: profiles.as_ref().map(profile_names).unwrap_or_default(),
        profile,
    })
}

pub fn follow_battery(system: &Connection, updates: Updates) -> Sender<BatteryCommand> {
    let (sender, receiver) = mpsc::channel();
    let reader = system.clone();
    let feed = updates.clone();
    spawn("battery", move || {
        let follow = || -> zbus::Result<()> {
            let rule = MatchRule::builder()
                .msg_type(message::Type::Signal)
                .interface("org.freedesktop.DBus.Properties")?
                .member("PropertiesChanged")?
                .path_namespace(UPOWER_PATH)?
                .build();
            let changes = MessageIterator::for_match_rule(rule, &reader, None)?;
            if !feed.send(Update::Battery(read(&reader))) {
                return Ok(());
            }
            for _ in changes.flatten() {
                if !feed.send(Update::Battery(read(&reader))) {
                    break;
                }
            }
            Ok(())
        };
        if let Err(error) = follow() {
            tracing::warn!("cannot follow the battery: {error}");
        }
    });
    let system = system.clone();
    spawn("battery-worker", move || {
        let Ok(profiles) = PowerProfilesProxyBlocking::new(&system) else {
            return;
        };
        for command in receiver {
            let BatteryCommand::Profile(profile) = command;
            if let Err(error) = profiles.set_active_profile(&profile) {
                updates.send(unexpected(error));
                updates.send(Update::Battery(read(&system)));
            }
        }
    });
    sender
}
