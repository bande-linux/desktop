#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use unbound_notify::{Notification, NotificationServer, Urgency};
use unbound_power::battery::{self, Alert, BatteryNotifier};
use unbound_test_bus::SessionBus;
use zbus::blocking::Connection;

const WAIT: Duration = Duration::from_secs(5);
const DEVICE_PATH: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
const NONE: u32 = 1;
const LOW: u32 = 3;
const CRITICAL: u32 = 4;
const ACTION: u32 = 5;

struct FakeDevice {
    warning_level: u32,
    percentage: f64,
}

#[zbus::interface(name = "org.freedesktop.UPower.Device")]
impl FakeDevice {
    #[zbus(property)]
    fn warning_level(&self) -> u32 {
        self.warning_level
    }

    #[zbus(property)]
    fn percentage(&self) -> f64 {
        self.percentage
    }
}

struct Fixture {
    upower: Connection,
    notifications: NotificationServer,
    _bus: SessionBus,
}

fn start(warning_level: u32, percentage: f64) -> Fixture {
    let bus = SessionBus::start().expect("dbus-daemon");
    let notifications = NotificationServer::start(&bus.connect().expect("notify connection"))
        .expect("notification server");
    let upower = bus.connect().expect("upower connection");
    upower
        .object_server()
        .at(
            DEVICE_PATH,
            FakeDevice {
                warning_level,
                percentage,
            },
        )
        .expect("device");
    upower
        .request_name("org.freedesktop.UPower")
        .expect("upower name");
    let power = bus.connect().expect("power connection");
    battery::watch(&power, Arc::new(BatteryNotifier::new(&power))).expect("watch");
    Fixture {
        upower,
        notifications,
        _bus: bus,
    }
}

impl Fixture {
    fn set(&self, warning_level: u32, percentage: f64) {
        let device = self
            .upower
            .object_server()
            .interface::<_, FakeDevice>(DEVICE_PATH)
            .expect("device");
        {
            let mut state = device.get_mut();
            state.percentage = percentage;
            state.warning_level = warning_level;
        }
        let state = device.get();
        zbus::block_on(async {
            state
                .percentage_changed(device.signal_emitter())
                .await
                .expect("signal");
            state
                .warning_level_changed(device.signal_emitter())
                .await
                .expect("signal");
        });
    }

    fn wait_for(&self, done: impl Fn(&[Notification]) -> bool) -> Vec<Notification> {
        let deadline = Instant::now() + WAIT;
        loop {
            let active = self.notifications.active();
            if done(&active) {
                return active;
            }
            assert!(Instant::now() < deadline, "still showing {active:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn upower_warning_levels_map_to_alerts() {
    assert_eq!(Alert::for_warning_level(NONE), None);
    assert_eq!(Alert::for_warning_level(2), None);
    assert_eq!(Alert::for_warning_level(LOW), Some(Alert::Low));
    assert_eq!(Alert::for_warning_level(CRITICAL), Some(Alert::Critical));
    assert_eq!(Alert::for_warning_level(ACTION), Some(Alert::Critical));
}

#[test]
fn a_low_battery_at_start_is_reported() {
    let fixture = start(LOW, 18.4);
    let active = fixture.wait_for(|active| active.len() == 1);
    assert_eq!(active[0].summary, "Low Battery");
    assert!(active[0].body.starts_with("18% "), "{}", active[0].body);
    assert_eq!(active[0].urgency, Urgency::Normal);
}

#[test]
fn a_critical_battery_replaces_the_low_alert_and_stays_on_screen() {
    let fixture = start(LOW, 18.0);
    let low = fixture.wait_for(|active| active.len() == 1)[0].id;

    fixture.set(CRITICAL, 4.0);
    let active = fixture.wait_for(|active| {
        active
            .iter()
            .any(|shown| shown.urgency == Urgency::Critical)
    });
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, low);
    assert_eq!(active[0].summary, "Battery Critically Low");
}

#[test]
fn connecting_power_closes_the_alert() {
    let fixture = start(CRITICAL, 4.0);
    fixture.wait_for(|active| active.len() == 1);
    fixture.set(NONE, 5.0);
    fixture.wait_for(<[Notification]>::is_empty);
}
