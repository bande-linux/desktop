#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bande_status::backlight::{DisplayLight, KeyboardLight};
use bande_test_bus::SessionBus;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;

const SESSION_PATH: &str = "/org/freedesktop/login1/session/_31";
const WAIT: Duration = Duration::from_secs(5);

type Calls = Arc<Mutex<Sender<(String, String, u32)>>>;

struct FakeManager;

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl FakeManager {
    fn get_session(&self, _session_id: String) -> zbus::fdo::Result<OwnedObjectPath> {
        OwnedObjectPath::try_from(SESSION_PATH)
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }
}

struct FakeSession {
    calls: Calls,
}

#[zbus::interface(name = "org.freedesktop.login1.Session")]
impl FakeSession {
    fn set_brightness(&self, subsystem: String, name: String, brightness: u32) {
        if let Ok(calls) = self.calls.lock() {
            let _ = calls.send((subsystem, name, brightness));
        }
    }
}

fn fake_logind(bus: &SessionBus) -> (Connection, Receiver<(String, String, u32)>) {
    let (sender, calls) = mpsc::channel();
    let connection = bus.connect().expect("logind connection");
    connection
        .object_server()
        .at("/org/freedesktop/login1", FakeManager)
        .expect("manager");
    connection
        .object_server()
        .at(
            SESSION_PATH,
            FakeSession {
                calls: Arc::new(Mutex::new(sender)),
            },
        )
        .expect("session");
    connection
        .request_name("org.freedesktop.login1")
        .expect("logind name");
    (connection, calls)
}

fn device(directory: &Path, files: &[(&str, &str)]) {
    std::fs::create_dir_all(directory).expect("device directory");
    for (name, text) in files {
        std::fs::write(directory.join(name), text).expect("attribute");
    }
}

#[test]
fn the_display_level_reads_sysfs_and_is_set_through_logind_never_fully_dark() {
    let class = tempfile::tempdir().expect("backlight class");
    device(
        &class.path().join("apple-panel-bl"),
        &[
            ("type", "firmware\n"),
            ("brightness", "300\n"),
            ("actual_brightness", "300\n"),
            ("max_brightness", "1200\n"),
        ],
    );
    let light = DisplayLight::find_in(class.path()).expect("a backlight");
    assert_eq!(light.level(), Some(0.25));

    let bus = SessionBus::start().expect("dbus-daemon");
    let (_logind, calls) = fake_logind(&bus);
    let client = bus.connect().expect("client");
    light.set_level(&client, 0.5).expect("set");
    assert_eq!(
        calls.recv_timeout(WAIT).expect("call"),
        ("backlight".to_owned(), "apple-panel-bl".to_owned(), 600)
    );
    light.set_level(&client, 0.0).expect("set");
    assert_eq!(calls.recv_timeout(WAIT).expect("call").2, 1);

    let mut seen = Vec::new();
    light
        .follow(|level| {
            seen.push(level);
            false
        })
        .expect("follow");
    assert_eq!(seen, [0.25]);
}

#[test]
fn the_keyboard_light_is_found_among_the_leds_and_may_turn_off() {
    let leds = tempfile::tempdir().expect("leds");
    device(
        &leds.path().join("input3::capslock"),
        &[("brightness", "1"), ("max_brightness", "1\n")],
    );
    device(
        &leds.path().join("kbd_backlight"),
        &[("brightness", "128"), ("max_brightness", "255\n")],
    );
    let light = KeyboardLight::find(leds.path()).expect("a keyboard backlight");
    assert_eq!(light.name(), "kbd_backlight");
    assert_eq!(light.level(), Some(128.0 / 255.0));
    assert_eq!(KeyboardLight::find(&leds.path().join("missing")), None);

    let bus = SessionBus::start().expect("dbus-daemon");
    let (_logind, calls) = fake_logind(&bus);
    light
        .set_level(&bus.connect().expect("client"), 0.0)
        .expect("set");
    assert_eq!(
        calls.recv_timeout(WAIT).expect("call"),
        ("leds".to_owned(), "kbd_backlight".to_owned(), 0)
    );
}
