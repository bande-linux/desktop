use std::sync::mpsc;
use std::thread;

use bande_bus::power_profiles::{POWER_SAVER, power_profiles};
use zbus::blocking::Connection;

use crate::battery::{DisplayDeviceProxyBlocking, LOW_WARNING_LEVEL, UPowerProxyBlocking};

#[zbus::proxy(
    interface = "org.bande.Config",
    default_service = "org.bande.Config",
    default_path = "/org/bande/Config"
)]
pub(crate) trait Config {
    #[zbus(property)]
    fn low_power_on_battery(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn lid_on_power(&self) -> zbus::Result<String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Situation {
    pub on_battery: bool,
    pub battery_low: bool,
    pub low_power_on_battery: bool,
}

#[derive(Debug, Default)]
pub struct AdapterPolicy {
    saved_profile: Option<String>,
}

impl AdapterPolicy {
    pub fn profile_for(&mut self, situation: Situation, current: &str) -> Option<String> {
        let saving =
            situation.on_battery && (situation.low_power_on_battery || situation.battery_low);
        match (&self.saved_profile, saving) {
            (None, true) if current != POWER_SAVER => {
                self.saved_profile = Some(current.to_owned());
                Some(POWER_SAVER.to_owned())
            }
            (Some(_), false) => {
                let saved = self.saved_profile.take()?;
                (current == POWER_SAVER).then_some(saved)
            }
            _ => None,
        }
    }
}

fn situation(
    upower: &UPowerProxyBlocking<'_>,
    battery: &DisplayDeviceProxyBlocking<'_>,
    config: &ConfigProxyBlocking<'_>,
) -> Situation {
    Situation {
        on_battery: upower.on_battery().unwrap_or(false),
        battery_low: battery
            .warning_level()
            .is_ok_and(|level| level >= LOW_WARNING_LEVEL),
        low_power_on_battery: config.low_power_on_battery().unwrap_or(false),
    }
}

pub fn follow(system: &Connection, session: &Connection) -> zbus::Result<()> {
    let upower = UPowerProxyBlocking::new(system)?;
    let battery = DisplayDeviceProxyBlocking::new(system)?;
    let config = ConfigProxyBlocking::new(session)?;
    let profiles = power_profiles(system)?;
    let (changed, changes) = mpsc::channel();
    let plugs = upower.receive_on_battery_changed();
    let levels = battery.receive_warning_level_changed();
    let low_power = config.receive_low_power_on_battery_changed();
    let notify = changed.clone();
    thread::spawn(move || {
        plugs.for_each(|_| {
            let _ = notify.send(());
        })
    });
    let notify = changed.clone();
    thread::spawn(move || {
        levels.for_each(|_| {
            let _ = notify.send(());
        })
    });
    thread::spawn(move || {
        low_power.for_each(|_| {
            let _ = changed.send(());
        })
    });
    thread::spawn(move || {
        let mut policy = AdapterPolicy::default();
        loop {
            let situation = situation(&upower, &battery, &config);
            if let Ok(current) = profiles.active_profile()
                && let Some(profile) = policy.profile_for(situation, &current)
                && let Err(error) = profiles.set_active_profile(&profile)
            {
                tracing::warn!("cannot switch to the {profile} power mode: {error}");
            }
            if changes.recv().is_err() {
                break;
            }
            changes.try_iter().count();
        }
    });
    Ok(())
}
