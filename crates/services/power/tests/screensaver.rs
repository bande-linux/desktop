#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{PipeReader, Read};
use std::os::fd::OwnedFd;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use unbound_power::screensaver::{BUS_NAME, InhibitionService};
use unbound_test_bus::SessionBus;
use zbus::blocking::Connection;

const WAIT: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(300);

struct Held {
    application: String,
    reason: String,
    released: Receiver<()>,
}

struct Fixture {
    _service: InhibitionService,
    held: Receiver<Held>,
    bus: SessionBus,
}

fn watch_release(reader: PipeReader) -> Receiver<()> {
    let (sender, released) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = reader;
        let mut byte = [0u8];
        if matches!(reader.read(&mut byte), Ok(0)) {
            let _ = sender.send(());
        }
    });
    released
}

fn start() -> Fixture {
    let bus = SessionBus::start().expect("dbus-daemon");
    let (sender, held) = mpsc::channel::<Held>();
    let sender: Arc<Mutex<Sender<Held>>> = Arc::new(Mutex::new(sender));
    let service = InhibitionService::start(
        &bus.connect().expect("service connection"),
        move |application, reason| {
            let (reader, writer) = std::io::pipe().ok()?;
            let _ = sender.lock().ok()?.send(Held {
                application: application.to_owned(),
                reason: reason.to_owned(),
                released: watch_release(reader),
            });
            Some(OwnedFd::from(writer))
        },
        |_| {},
    )
    .expect("inhibition service");
    Fixture {
        _service: service,
        held,
        bus,
    }
}

fn inhibit(client: &Connection, path: &str) -> u32 {
    client
        .call_method(
            Some(BUS_NAME),
            path,
            Some(BUS_NAME),
            "Inhibit",
            &("Videos", "Playing a film"),
        )
        .expect("Inhibit")
        .body()
        .deserialize()
        .expect("cookie")
}

fn uninhibit(client: &Connection, cookie: u32) {
    client
        .call_method(
            Some(BUS_NAME),
            "/org/freedesktop/ScreenSaver",
            Some(BUS_NAME),
            "UnInhibit",
            &cookie,
        )
        .expect("UnInhibit");
}

#[test]
fn inhibit_holds_an_inhibitor_until_uninhibit() {
    let fixture = start();
    let client = fixture.bus.connect().expect("client");
    let cookie = inhibit(&client, "/org/freedesktop/ScreenSaver");

    let held = fixture.held.recv_timeout(WAIT).expect("held");
    assert_eq!(
        (held.application.as_str(), held.reason.as_str()),
        ("Videos", "Playing a film")
    );
    assert!(held.released.recv_timeout(QUIET).is_err());

    uninhibit(&client, cookie);
    held.released.recv_timeout(WAIT).expect("released");
}

#[test]
fn the_legacy_object_path_is_served_too() {
    let fixture = start();
    let client = fixture.bus.connect().expect("client");
    inhibit(&client, "/ScreenSaver");
    fixture.held.recv_timeout(WAIT).expect("held");
}

#[test]
fn cookies_are_distinct() {
    let fixture = start();
    let client = fixture.bus.connect().expect("client");
    let first = inhibit(&client, "/org/freedesktop/ScreenSaver");
    let second = inhibit(&client, "/org/freedesktop/ScreenSaver");
    assert_ne!(first, second);
}

#[test]
fn another_client_cannot_release_an_inhibitor() {
    let fixture = start();
    let owner = fixture.bus.connect().expect("owner");
    let cookie = inhibit(&owner, "/org/freedesktop/ScreenSaver");
    let held = fixture.held.recv_timeout(WAIT).expect("held");

    uninhibit(&fixture.bus.connect().expect("stranger"), cookie);
    assert!(held.released.recv_timeout(QUIET).is_err());
}

#[test]
fn an_inhibitor_ends_when_its_client_leaves_the_bus() {
    let fixture = start();
    let client = fixture.bus.connect().expect("client");
    inhibit(&client, "/org/freedesktop/ScreenSaver");
    let held = fixture.held.recv_timeout(WAIT).expect("held");

    drop(client);
    held.released.recv_timeout(WAIT).expect("released");
}
