#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Role, TreeId};
use support::{WAIT, find, has_text};
use unbound_settings::{
    BatteryCommand, Charge, Command, PaneId, SettingId, SettingsView, Value, bucket_levels,
    charge_of, charge_text, feed, follow_battery,
};
use unbound_test_bus::SessionBus;
use unbound_toolkit::{Palette, Scheme, TextSystem};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const BATTERY: &str = "/org/freedesktop/UPower/devices/battery_BAT0";

struct UPower;

#[zbus::interface(name = "org.freedesktop.UPower")]
impl UPower {
    fn enumerate_devices(&self) -> Vec<OwnedObjectPath> {
        vec![OwnedObjectPath::try_from(BATTERY).expect("path")]
    }
}

struct Device;

#[zbus::interface(name = "org.freedesktop.UPower.Device")]
impl Device {
    #[zbus(property, name = "Type")]
    fn kind(&self) -> u32 {
        2
    }

    #[zbus(property)]
    fn power_supply(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn is_present(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn percentage(&self) -> f64 {
        64.0
    }

    #[zbus(property)]
    fn state(&self) -> u32 {
        2
    }

    #[zbus(property)]
    fn time_to_empty(&self) -> i64 {
        3 * 3600 + 20 * 60
    }

    #[zbus(property)]
    fn time_to_full(&self) -> i64 {
        0
    }

    fn get_history(&self, _kind: &str, _span: u32, _resolution: u32) -> Vec<(u32, f64, u32)> {
        vec![
            (1_000_000, 90.0, 2),
            (1_040_000, 75.0, 2),
            (1_086_400, 64.0, 2),
        ]
    }
}

struct Profiles(Arc<Mutex<String>>);

#[zbus::interface(name = "org.freedesktop.UPower.PowerProfiles")]
impl Profiles {
    #[zbus(property)]
    fn active_profile(&self) -> String {
        self.0.lock().expect("profile").clone()
    }

    #[zbus(property)]
    async fn set_active_profile(
        &mut self,
        profile: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        *self.0.lock().expect("profile") = profile;
        let _ = self.active_profile_changed(&emitter).await;
    }

    #[zbus(property)]
    fn profiles(&self) -> Vec<HashMap<String, OwnedValue>> {
        ["power-saver", "balanced"]
            .into_iter()
            .map(|name| {
                HashMap::from([(
                    "Profile".to_owned(),
                    OwnedValue::try_from(zbus::zvariant::Value::from(name)).expect("value"),
                )])
            })
            .collect()
    }
}

#[test]
fn the_battery_pane_shows_the_charge_and_switches_power_modes() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let profile = Arc::new(Mutex::new("balanced".to_owned()));
    let _daemon = zbus::blocking::connection::Builder::address(bus.address())
        .expect("address")
        .name("org.freedesktop.UPower")
        .expect("name")
        .name("org.freedesktop.UPower.PowerProfiles")
        .expect("name")
        .serve_at("/org/freedesktop/UPower", UPower)
        .expect("serve")
        .serve_at("/org/freedesktop/UPower/devices/DisplayDevice", Device)
        .expect("serve")
        .serve_at(BATTERY, Device)
        .expect("serve")
        .serve_at(
            "/org/freedesktop/UPower/PowerProfiles",
            Profiles(Arc::clone(&profile)),
        )
        .expect("serve")
        .build()
        .expect("fake UPower");
    let system = bus.connect().expect("connection");
    let (mut feed, updates) = feed().expect("feed");
    let sender = follow_battery(&system, updates);
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    let deadline = Instant::now() + WAIT;
    while view.value(SettingId::PowerMode).is_none() {
        for update in feed.drain() {
            view.apply(update);
        }
        assert!(Instant::now() < deadline, "no battery");
        std::thread::sleep(Duration::from_millis(20));
    }
    view.select_pane(PaneId::Power);
    assert!(find(&mut view, Role::Row, "64%, On battery, 3 h 20 min left").is_some());
    assert!(has_text(&mut view, "64% · On battery, 3 h 20 min left"));
    assert!(
        find(
            &mut view,
            Role::Image,
            "Battery level over the last 24 hours: 90% → 64%"
        )
        .is_some()
    );
    assert_eq!(
        view.value(SettingId::PowerMode),
        Some(&Value::Keyword("balanced".to_owned()))
    );
    let saver = find(&mut view, Role::RadioButton, "Power Saver").expect("Power Saver");
    assert!(
        find(&mut view, Role::RadioButton, "Performance").is_none(),
        "a mode the computer lacks is not offered"
    );
    view.perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: saver,
        data: None,
    });
    let commands = view.take_commands();
    assert_eq!(
        commands,
        [Command::Battery(BatteryCommand::Profile(
            "power-saver".to_owned()
        ))]
    );
    for command in commands {
        if let Command::Battery(command) = command {
            sender.send(command).expect("battery worker");
        }
    }
    let deadline = Instant::now() + WAIT;
    while *profile.lock().expect("profile") != "power-saver" {
        assert!(Instant::now() < deadline, "the mode never changed");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_charge_reads_as_time_left_or_until_full() {
    assert_eq!(
        charge_of(2, 5400, 0),
        Charge::Discharging { minutes: Some(90) }
    );
    assert_eq!(
        charge_of(1, 0, 1200),
        Charge::Charging { minutes: Some(20) }
    );
    assert_eq!(charge_of(4, 0, 0), Charge::Full);
    assert_eq!(charge_of(2, 0, 0), Charge::Discharging { minutes: None });
    assert_eq!(
        charge_text(Charge::Charging { minutes: Some(80) }),
        "Charging, 1 h 20 min until full"
    );
    assert_eq!(
        charge_text(Charge::Discharging { minutes: Some(45) }),
        "On battery, 45 min left"
    );
}

#[test]
fn the_history_keeps_the_last_level_in_each_half_hour() {
    let levels = bucket_levels(&[(0, 100.0), (3600, 80.0), (7200, 50.0)], 4, 7200);
    assert_eq!(levels, [Some(100.0), Some(80.0), Some(80.0), Some(50.0)]);
    let levels = bucket_levels(&[(10_000, 40.0)], 2, 7200);
    assert_eq!(levels, [None, Some(40.0)]);
    assert_eq!(bucket_levels(&[], 2, 7200), [None, None]);
}
