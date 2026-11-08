use std::sync::mpsc::{self, Sender};

use bande_status::history::{PowerSource, UPower, charge_history, system_battery};
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::OwnedValue;
use zbus::{MatchRule, message};

use bande_settings_panes::SettingId;

use crate::failure::failed;
use crate::follow::{Update, Updates, spawn};

const UPOWER_PATH: &str = "/org/freedesktop/UPower";
const PLACEHOLDER_DRIVER: &str = "placeholder";
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
    pub capacity: Option<f64>,
    pub limit: Option<ChargeLimit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChargeLimit {
    pub enabled: bool,
    pub end: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Profiles {
    Missing,
    Offered(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PowerState {
    pub battery: Option<BatteryState>,
    pub on_battery: bool,
    pub lid: bool,
    pub profiles: Profiles,
    pub profile: Option<String>,
}

impl PowerState {
    pub fn offered(&self) -> &[String] {
        match &self.profiles {
            Profiles::Offered(names) => names,
            Profiles::Missing => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatteryCommand {
    Profile(String),
    ChargeLimit(bool),
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

fn text(profile: &std::collections::HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    String::try_from(profile.get(key)?.try_clone().ok()?).ok()
}

fn profile_names(profiles: &PowerProfilesProxyBlocking<'_>) -> Profiles {
    match profiles.profiles() {
        Ok(found) => Profiles::Offered(
            found
                .iter()
                .filter(|profile| text(profile, "Driver").as_deref() != Some(PLACEHOLDER_DRIVER))
                .filter_map(|profile| text(profile, "Profile"))
                .collect(),
        ),
        Err(error) => {
            tracing::info!("no power modes: {error}");
            Profiles::Missing
        }
    }
}

fn read_battery(system: &Connection, source: Option<&PowerSource<'_>>) -> Option<BatteryState> {
    let display = DeviceProxyBlocking::builder(system)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .ok()?;
    if !display.is_present().ok()? {
        return None;
    }
    let history = source
        .and_then(|source| charge_history(source, HISTORY_SECONDS, HISTORY_POINTS).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|sample| (sample.at, sample.percent))
        .collect();
    let capacity = source
        .and_then(|source| source.capacity().ok())
        .filter(|capacity| *capacity > 0.0);
    let limit = source
        .filter(|source| source.charge_threshold_supported().unwrap_or(false))
        .map(|source| ChargeLimit {
            enabled: source.charge_threshold_enabled().unwrap_or(false),
            end: source
                .charge_end_threshold()
                .ok()
                .filter(|end| (1..100).contains(end)),
        });
    Some(BatteryState {
        percent: display.percentage().ok()?,
        charge: charge_of(
            display.state().unwrap_or(0),
            display.time_to_empty().unwrap_or(0),
            display.time_to_full().unwrap_or(0),
        ),
        history,
        capacity,
        limit,
    })
}

fn read(system: &Connection) -> Option<PowerState> {
    let upower = UPower::builder(system)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .ok()?;
    let on_battery = match upower.on_battery() {
        Ok(on_battery) => on_battery,
        Err(error) => {
            tracing::warn!("cannot read the power source: {error}");
            return None;
        }
    };
    let source = system_battery(system).ok().flatten();
    let profiles = PowerProfilesProxyBlocking::builder(system)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .ok();
    Some(PowerState {
        battery: read_battery(system, source.as_ref()),
        on_battery,
        lid: upower.lid_is_present().unwrap_or(false),
        profiles: profiles.as_ref().map_or(Profiles::Missing, profile_names),
        profile: profiles
            .as_ref()
            .and_then(|profiles| profiles.active_profile().ok()),
    })
}

fn change(system: &Connection, command: BatteryCommand) -> zbus::Result<()> {
    match command {
        BatteryCommand::Profile(profile) => {
            PowerProfilesProxyBlocking::new(system)?.set_active_profile(&profile)
        }
        BatteryCommand::ChargeLimit(enabled) => {
            let battery = system_battery(system)?
                .ok_or_else(|| zbus::Error::Failure("no battery".to_owned()))?;
            battery.enable_charge_threshold(enabled)
        }
    }
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
            if !feed.send(Update::Power(read(&reader))) {
                return Ok(());
            }
            for _ in changes.flatten() {
                if !feed.send(Update::Power(read(&reader))) {
                    break;
                }
            }
            Ok(())
        };
        if let Err(error) = follow() {
            tracing::warn!("cannot follow the power source: {error}");
            feed.send(Update::Power(None));
        }
    });
    let system = system.clone();
    spawn("battery-worker", move || {
        for command in receiver {
            let setting = match command {
                BatteryCommand::Profile(_) => (SettingId::PowerMode, "service-power"),
                BatteryCommand::ChargeLimit(_) => (SettingId::BatteryLevel, "service-battery"),
            };
            if let Err(error) = change(&system, command) {
                updates.send(failed(setting.0, &error, setting.1));
                updates.send(Update::Power(read(&system)));
            }
        }
    });
    sender
}
