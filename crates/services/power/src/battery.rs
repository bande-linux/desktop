use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use unbound_notify::Urgency;
use unbound_notify::client::{self, Request};
use zbus::blocking::Connection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alert {
    Low,
    Critical,
}

impl Alert {
    pub fn for_warning_level(warning_level: u32) -> Option<Self> {
        match warning_level {
            3 => Some(Self::Low),
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
    shown: Mutex<Option<(Alert, u32)>>,
}

impl BatteryNotifier {
    pub fn new(session: &Connection) -> Self {
        Self {
            session: session.clone(),
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
        let remaining = format!("{}% of the battery remains.", percentage.round());
        let (summary, body, urgency) = match alert {
            Alert::Low => (
                "Low Battery",
                format!("{remaining} Connect to power soon."),
                Urgency::Normal,
            ),
            Alert::Critical => (
                "Battery Critically Low",
                format!("{remaining} Connect to power now, or the computer will sleep."),
                Urgency::Critical,
            ),
        };
        let id = client::notify(
            &self.session,
            &Request {
                app_name: "Power",
                replaces_id: shown.map_or(0, |(_, id)| id),
                app_icon: "battery-caution",
                summary,
                body: &body,
                actions: &[],
                urgency,
            },
        )?;
        *shown = Some((alert, id));
        Ok(())
    }
}

pub fn watch(system: &Connection, notifier: Arc<BatteryNotifier>) -> zbus::Result<()> {
    let device = DisplayDeviceProxyBlocking::new(system)?;
    let changes = device.receive_warning_level_changed();
    let report = move |device: &DisplayDeviceProxyBlocking<'_>| {
        if let (Ok(level), Ok(percentage)) = (device.warning_level(), device.percentage())
            && let Err(error) = notifier.update(level, percentage)
        {
            eprintln!("unbound-power: cannot post the battery notification: {error}");
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
