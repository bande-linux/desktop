use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use bande_l10n::tr;
use bande_notify::Urgency;
use bande_notify::client::{self, Request};
use zbus::blocking::Connection;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Alert {
    Low,
    Critical,
}

pub const LOW_WARNING_LEVEL: u32 = 3;

impl Alert {
    pub fn for_warning_level(warning_level: u32) -> Option<Self> {
        match warning_level {
            LOW_WARNING_LEVEL => Some(Self::Low),
            4 | 5 => Some(Self::Critical),
            _ => None,
        }
    }
}

#[zbus::proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower/devices/DisplayDevice"
)]
pub trait DisplayDevice {
    #[zbus(property)]
    fn warning_level(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;
}

pub struct BatteryNotifier {
    session: Connection,
    system: Connection,
    shown: Mutex<Option<(Alert, u32)>>,
}

impl BatteryNotifier {
    pub fn new(session: &Connection, system: &Connection) -> Self {
        Self {
            session: session.clone(),
            system: system.clone(),
            shown: Mutex::new(None),
        }
    }

    pub fn update(&self, warning_level: u32, percentage: f64) -> zbus::Result<()> {
        let mut shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
        let alert = Alert::for_warning_level(warning_level);
        if alert == shown.map(|(alert, _)| alert) {
            return Ok(());
        }
        let Some(alert) = alert else {
            if let Some((_, id)) = shown.take() {
                client::close(&self.session, id)?;
            }
            return Ok(());
        };
        let percentage = percentage.round();
        let (summary, body, urgency) = match alert {
            Alert::Low => (
                tr!("Low Battery"),
                tr!(
                    "{percentage}% of the battery remains. Connect to power soon.",
                    percentage = percentage
                ),
                Urgency::Normal,
            ),
            Alert::Critical => (
                tr!("Battery Critically Low"),
                self.critical_warning(percentage),
                Urgency::Critical,
            ),
        };
        let app_name = tr!("Power");
        let id = client::notify(
            &self.session,
            &Request {
                app_name: &app_name,
                replaces_id: shown.map_or(0, |(_, id)| id),
                app_icon: "battery-caution",
                summary: &summary,
                body: &body,
                actions: &[],
                urgency,
                sound: Some("battery-low"),
                desktop_entry: None,
            },
        )?;
        *shown = Some((alert, id));
        Ok(())
    }

    fn critical_warning(&self, percentage: f64) -> String {
        let action = UPowerProxyBlocking::new(&self.system)
            .and_then(|upower| upower.get_critical_action())
            .inspect_err(|error| tracing::warn!("cannot ask what an empty battery does: {error}"))
            .unwrap_or_default();
        match action.as_str() {
            "PowerOff" => tr!(
                "{percentage}% of the battery remains. Connect to power now, or the computer will shut down and unsaved work will be lost.",
                percentage = percentage
            ),
            "Hibernate" | "HybridSleep" => tr!(
                "{percentage}% of the battery remains. Connect to power now, or the computer will hibernate.",
                percentage = percentage
            ),
            "Suspend" => tr!(
                "{percentage}% of the battery remains. Connect to power now, or the computer will sleep.",
                percentage = percentage
            ),
            _ => tr!(
                "{percentage}% of the battery remains. Connect to power now.",
                percentage = percentage
            ),
        }
    }
}

#[zbus::proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
pub trait UPower {
    #[zbus(property)]
    fn on_battery(&self) -> zbus::Result<bool>;

    fn get_critical_action(&self) -> zbus::Result<String>;
}

pub fn chime_when_plugged_in(system: &Connection, session: &Connection) -> zbus::Result<()> {
    let upower = UPowerProxyBlocking::new(system)?;
    let changes = upower.receive_on_battery_changed();
    let mut on_battery = upower.on_battery().unwrap_or(false);
    let session = session.clone();
    thread::spawn(move || {
        for change in changes {
            let Ok(now) = change.get() else {
                continue;
            };
            if on_battery && !now {
                bande_sound::play(&session, "device-added");
            }
            on_battery = now;
        }
    });
    Ok(())
}

pub fn watch(system: &Connection, notifier: Arc<BatteryNotifier>) -> zbus::Result<()> {
    let device = DisplayDeviceProxyBlocking::new(system)?;
    let changes = device.receive_warning_level_changed();
    let report = move |device: &DisplayDeviceProxyBlocking<'_>| {
        if let (Ok(level), Ok(percentage)) = (device.warning_level(), device.percentage())
            && let Err(error) = notifier.update(level, percentage)
        {
            tracing::warn!("cannot post the battery notification: {error}");
        }
    };
    report(&device);
    thread::spawn(move || {
        for _ in changes {
            report(&device);
        }
    });
    Ok(())
}
