#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use unbound_power::battery;
use unbound_test_bus::SessionBus;
use zbus::blocking::Connection;

const UPOWER_PATH: &str = "/org/freedesktop/UPower";

struct FakeUPower {
    on_battery: bool,
}

#[zbus::interface(name = "org.freedesktop.UPower")]
impl FakeUPower {
    #[zbus(property)]
    fn on_battery(&self) -> bool {
        self.on_battery
    }
}

struct Sounds(Mutex<Sender<String>>);

#[zbus::interface(name = "org.unbound.Sound")]
impl Sounds {
    fn play(&self, name: &str) {
        let _ = self.0.lock().unwrap().send(name.to_owned());
    }
}

fn set(upower: &Connection, on_battery: bool) {
    let fake = upower
        .object_server()
        .interface::<_, FakeUPower>(UPOWER_PATH)
        .expect("upower");
    fake.get_mut().on_battery = on_battery;
    zbus::block_on(fake.get().on_battery_changed(fake.signal_emitter())).expect("signal");
}

fn start() -> (SessionBus, Connection, Connection, Receiver<String>) {
    let bus = SessionBus::start().expect("dbus-daemon");
    let upower = zbus::blocking::connection::Builder::address(bus.address())
        .expect("address")
        .name("org.freedesktop.UPower")
        .expect("name")
        .serve_at(UPOWER_PATH, FakeUPower { on_battery: true })
        .expect("serve")
        .build()
        .expect("upower");
    let (sender, played) = mpsc::channel();
    let sounds = zbus::blocking::connection::Builder::address(bus.address())
        .expect("address")
        .name("org.unbound.Sound")
        .expect("name")
        .serve_at("/org/unbound/Sound", Sounds(Mutex::new(sender)))
        .expect("serve")
        .build()
        .expect("sounds");
    let power = bus.connect().expect("power");
    battery::chime_when_plugged_in(&power, &power).expect("watch");
    (bus, upower, sounds, played)
}

#[test]
fn plugging_in_power_chimes_once_and_unplugging_is_silent() {
    let (_bus, upower, _sounds, played) = start();
    assert!(played.recv_timeout(Duration::from_millis(300)).is_err());

    set(&upower, false);
    assert_eq!(
        played.recv_timeout(Duration::from_secs(5)).as_deref(),
        Ok("device-added")
    );

    set(&upower, true);
    assert!(played.recv_timeout(Duration::from_millis(400)).is_err());
}
