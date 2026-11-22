use std::collections::HashMap;
use std::thread;

use bande_bluetooth::{Bluetooth, Device, change_rules, shown_change};
use bande_l10n::tr;
use bande_notify::Urgency;
use bande_notify::client::{self, Request};
use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, MessageIterator};

use crate::battery::Alert;

const LOW: u8 = 15;
const CRITICAL: u8 = 5;

fn alert(level: u8) -> Option<Alert> {
    if level <= CRITICAL {
        Some(Alert::Critical)
    } else if level <= LOW {
        Some(Alert::Low)
    } else {
        None
    }
}

struct Warnings {
    session: Connection,
    shown: HashMap<String, (Alert, u32)>,
}

impl Warnings {
    fn check(&mut self, device: &Device) {
        let Some(level) = device.battery.filter(|_| device.connected) else {
            return;
        };
        let shown = self.shown.get(&device.address).copied();
        let Some(alert) = alert(level) else {
            if let Some((_, id)) = self.shown.remove(&device.address)
                && let Err(error) = client::close(&self.session, id)
            {
                tracing::debug!("cannot close the battery notification: {error}");
            }
            return;
        };
        if shown.is_some_and(|(shown, _)| shown >= alert) {
            return;
        }
        let name = device.name.clone();
        let summary = match alert {
            Alert::Low => tr!("{name} Battery Low", name = name),
            Alert::Critical => tr!("{name} Battery Almost Empty", name = name),
        };
        let body = match alert {
            Alert::Low => tr!("{level}% remains. Charge it soon.", level = level),
            Alert::Critical => tr!("{level}% remains. Charge it now.", level = level),
        };
        let posted = client::notify(
            &self.session,
            &Request {
                app_name: &tr!("Bluetooth"),
                replaces_id: shown.map_or(0, |(_, id)| id),
                app_icon: device.kind.symbol(),
                summary: &summary,
                body: &body,
                actions: &[],
                urgency: Urgency::Normal,
                sound: Some("battery-low"),
                desktop_entry: None,
            },
        );
        match posted {
            Ok(id) => {
                self.shown.insert(device.address.clone(), (alert, id));
            }
            Err(error) => tracing::warn!("cannot warn that {name} runs low: {error}"),
        }
    }

    fn check_all(&mut self, bluetooth: &Bluetooth) {
        match bluetooth.radio() {
            Ok(Some(radio)) => radio.paired.iter().for_each(|device| self.check(device)),
            Ok(None) => {}
            Err(error) => tracing::debug!("cannot read the Bluetooth batteries: {error}"),
        }
    }
}

pub fn watch(system: &Connection, session: &Connection) -> zbus::Result<()> {
    let messages = MessageIterator::from(system);
    let bus = DBusProxy::new(system)?;
    for rule in change_rules()? {
        bus.add_match_rule(rule)?;
    }
    let bluetooth = Bluetooth::new(system);
    let mut warnings = Warnings {
        session: session.clone(),
        shown: HashMap::new(),
    };
    thread::Builder::new()
        .name("bluetooth-batteries".to_owned())
        .spawn(move || {
            warnings.check_all(&bluetooth);
            for message in messages.flatten() {
                if shown_change(&message) {
                    warnings.check_all(&bluetooth);
                }
            }
        })?;
    Ok(())
}
