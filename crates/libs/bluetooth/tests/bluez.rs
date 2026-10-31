#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use unbound_bluetooth::{Bluetooth, PairingPrompt, register_agent};
use unbound_test_bus::SessionBus;
use unbound_test_bus::bluez::{FakeBluez, HEADPHONES, MOUSE, PHONE};

#[derive(Default)]
struct Prompt {
    accept: bool,
    asked: Mutex<Vec<String>>,
}

impl PairingPrompt for Prompt {
    fn confirm_passkey(&self, device: &str, passkey: u32) -> bool {
        self.asked
            .lock()
            .expect("asked")
            .push(format!("confirm {device} {passkey:06}"));
        self.accept
    }

    fn show_code(&self, device: &str, code: &str) {
        self.asked
            .lock()
            .expect("asked")
            .push(format!("show {device} {code}"));
    }

    fn ask_pin_code(&self, _device: &str) -> Option<String> {
        None
    }

    fn authorize(&self, _device: &str) -> bool {
        self.accept
    }

    fn dismiss(&self) {}
}

struct Fixture {
    fake: FakeBluez,
    prompt: Arc<Prompt>,
    client: Bluetooth,
    _bus: SessionBus,
}

fn start(accept: bool) -> Fixture {
    let bus = SessionBus::start().expect("dbus-daemon");
    let fake = FakeBluez::start(&bus).expect("fake BlueZ");
    let client_bus = bus.connect().expect("client connection");
    let prompt = Arc::new(Prompt {
        accept,
        ..Prompt::default()
    });
    register_agent(&client_bus, prompt.clone()).expect("agent");
    Fixture {
        fake,
        prompt,
        client: Bluetooth::new(&client_bus),
        _bus: bus,
    }
}

impl Fixture {
    fn named(&self, name: &str) -> unbound_bluetooth::Device {
        self.client
            .paired_devices()
            .expect("paired")
            .into_iter()
            .chain(self.client.nearby_devices().expect("nearby"))
            .find(|device| device.name == name)
            .expect("device listed")
    }

    fn asked(&self) -> Vec<String> {
        self.prompt.asked.lock().expect("asked").clone()
    }
}

#[test]
fn without_bluez_running_bluetooth_is_unavailable_and_nothing_is_started() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let client = Bluetooth::new(&bus.connect().expect("client connection"));
    assert!(!client.available().expect("available"));
    assert!(!client.powered().expect("powered"));
    assert!(client.paired_devices().expect("paired").is_empty());
}

#[test]
fn turning_bluetooth_off_and_on_writes_the_adapter_power() {
    let fixture = start(true);
    assert!(fixture.client.available().expect("available"));
    assert!(fixture.client.powered().expect("powered"));
    fixture.client.set_powered(false).expect("off");
    assert!(!fixture.fake.powered().expect("adapter"));
    fixture.client.set_powered(true).expect("on");
    assert!(fixture.fake.powered().expect("adapter"));
}

#[test]
fn paired_devices_list_connected_first_then_by_name() {
    let fixture = start(true);
    let paired: Vec<(String, bool, Option<String>)> = fixture
        .client
        .paired_devices()
        .expect("paired")
        .into_iter()
        .map(|device| (device.name, device.connected, device.icon))
        .collect();
    assert_eq!(
        paired,
        [
            (
                "Studio Headphones".to_owned(),
                true,
                Some("audio-headphones".to_owned())
            ),
            ("mouse".to_owned(), false, Some("input-mouse".to_owned())),
        ]
    );
    let nearby: Vec<String> = fixture
        .client
        .nearby_devices()
        .expect("nearby")
        .into_iter()
        .map(|device| device.address)
        .collect();
    assert_eq!(nearby, ["E8:06:88:0A:0B:0C", "5C:F9:38:07:08:09"]);
}

#[test]
fn connecting_and_disconnecting_a_paired_device() {
    let fixture = start(true);
    fixture
        .client
        .connect(&fixture.named("mouse"))
        .expect("connect");
    assert!(fixture.fake.device(MOUSE).expect("mouse").connected);
    fixture
        .client
        .disconnect(&fixture.named("Studio Headphones"))
        .expect("disconnect");
    assert!(
        !fixture
            .fake
            .device(HEADPHONES)
            .expect("headphones")
            .connected
    );
}

#[test]
fn a_device_reports_its_battery_until_it_is_forgotten() {
    let fixture = start(true);
    let headphones = fixture.named("Studio Headphones");
    assert_eq!(headphones.battery, Some(80));
    assert_eq!(fixture.named("mouse").battery, None);
    fixture.client.forget(&headphones).expect("forget");
    assert!(!fixture.fake.has_device(HEADPHONES));
    let paired: Vec<String> = fixture
        .client
        .paired_devices()
        .expect("paired")
        .into_iter()
        .map(|device| device.name)
        .collect();
    assert_eq!(paired, ["mouse"]);
}

#[test]
fn discovery_runs_only_while_asked() {
    let fixture = start(true);
    fixture.client.set_discovering(true).expect("start");
    assert!(fixture.fake.discovering().expect("adapter"));
    fixture.client.set_discovering(false).expect("stop");
    assert!(!fixture.fake.discovering().expect("adapter"));
}

#[test]
fn pairing_confirms_the_passkey_then_trusts_and_connects() {
    let fixture = start(true);
    assert_eq!(fixture.fake.capability(), "KeyboardDisplay");
    fixture.client.pair(&fixture.named("Phone")).expect("pair");
    assert_eq!(fixture.asked(), ["confirm Phone 123456"]);
    let phone = fixture.fake.device(PHONE).expect("phone");
    assert!(phone.paired && phone.trusted && phone.connected);
    assert!(
        fixture
            .client
            .paired_devices()
            .expect("paired")
            .iter()
            .any(|device| device.name == "Phone")
    );
}

#[test]
fn a_keyboard_shows_its_passkey_as_six_digits() {
    let fixture = start(true);
    fixture
        .client
        .pair(&fixture.named("Keyboard"))
        .expect("pair");
    assert_eq!(fixture.asked(), ["show Keyboard 004207"]);
}

#[test]
fn declining_the_passkey_leaves_the_device_unpaired() {
    let fixture = start(false);
    assert!(fixture.client.pair(&fixture.named("Phone")).is_err());
    let phone = fixture.fake.device(PHONE).expect("phone");
    assert!(!phone.paired && !phone.trusted && !phone.connected);
}
