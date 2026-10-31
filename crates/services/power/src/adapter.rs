use std::os::fd::OwnedFd;
use std::sync::mpsc;
use std::thread;

use unbound_config::client::ConfigProxy;
use zbus::blocking::Connection;

use crate::L10N;
use crate::logind;

pub const POWER_SAVER: &str = "power-saver";
const KEEP_RUNNING: &str = "keep-running";

#[zbus::proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
trait UPower {
    #[zbus(property)]
    fn on_battery(&self) -> zbus::Result<bool>;
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Situation {
    pub on_battery: bool,
    pub low_power_on_battery: bool,
    pub keep_running_with_lid_closed: bool,
}

#[derive(Debug, Default)]
pub struct AdapterPolicy {
    saved_profile: Option<String>,
}

impl AdapterPolicy {
    pub fn profile_for(&mut self, situation: Situation, current: &str) -> Option<String> {
        let saving = situation.on_battery && situation.low_power_on_battery;
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

    pub fn holds_lid(situation: Situation) -> bool {
        !situation.on_battery && situation.keep_running_with_lid_closed
    }
}

fn situation(upower: &UPowerProxyBlocking<'_>, config: &ConfigProxy<'_>) -> Situation {
    Situation {
        on_battery: upower.on_battery().unwrap_or(false),
        low_power_on_battery: config.low_power_on_battery().unwrap_or(false),
        keep_running_with_lid_closed: config
            .lid_on_power()
            .is_ok_and(|action| action == KEEP_RUNNING),
    }
}

pub fn follow(system: &Connection, session: &Connection) -> zbus::Result<()> {
    let upower = UPowerProxyBlocking::new(system)?;
    let config = ConfigProxy::new(session)?;
    let profiles = PowerProfilesProxyBlocking::new(system)?;
    let (changed, changes) = mpsc::channel();
    let battery = upower.receive_on_battery_changed();
    let low_power = config.receive_low_power_on_battery_changed();
    let lid = config.receive_lid_on_power_changed();
    let notify = changed.clone();
    thread::spawn(move || {
        battery.for_each(|_| {
            let _ = notify.send(());
        })
    });
    let notify = changed.clone();
    thread::spawn(move || {
        low_power.for_each(|_| {
            let _ = notify.send(());
        })
    });
    thread::spawn(move || {
        lid.for_each(|_| {
            let _ = changed.send(());
        })
    });
    let system = system.clone();
    thread::spawn(move || {
        let mut policy = AdapterPolicy::default();
        let mut lid_hold: Option<OwnedFd> = None;
        loop {
            let situation = situation(&upower, &config);
            if let Ok(current) = profiles.active_profile()
                && let Some(profile) = policy.profile_for(situation, &current)
                && let Err(error) = profiles.set_active_profile(&profile)
            {
                tracing::warn!("cannot switch to the {profile} power mode: {error}");
            }
            match (AdapterPolicy::holds_lid(situation), lid_hold.is_some()) {
                (true, false) => {
                    lid_hold = logind::inhibit(
                        &system,
                        "handle-lid-switch",
                        &L10N.tr("app-name"),
                        &L10N.tr("keep-running-with-lid-closed"),
                        "block",
                    )
                    .inspect_err(|error| tracing::warn!("cannot keep running: {error}"))
                    .ok();
                }
                (false, true) => lid_hold = None,
                _ => {}
            }
            if changes.recv().is_err() {
                break;
            }
            changes.try_iter().count();
        }
    });
    Ok(())
}
