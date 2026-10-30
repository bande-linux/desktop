#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use unbound_notify::NotificationServer;
use unbound_notify::client::NotificationCenter;
use unbound_test_bus::SessionBus;
use zbus::blocking::Connection;
use zbus::zvariant::Value;

const WAIT: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(400);

struct Sounds(Mutex<Sender<String>>);

#[zbus::interface(name = "org.unbound.Sound")]
impl Sounds {
    fn play(&self, name: &str) {
        let _ = self.0.lock().unwrap().send(name.to_owned());
    }
}

struct Fixture {
    _bus: SessionBus,
    _server: NotificationServer,
    _sounds: Connection,
    client: Connection,
    played: Receiver<String>,
}

fn start() -> Fixture {
    let bus = SessionBus::start().expect("dbus-daemon");
    let server = NotificationServer::start(&bus.connect().expect("server")).expect("server");
    let (sender, played) = mpsc::channel();
    let sounds = zbus::blocking::connection::Builder::address(bus.address())
        .expect("address")
        .name("org.unbound.Sound")
        .expect("name")
        .serve_at("/org/unbound/Sound", Sounds(Mutex::new(sender)))
        .expect("serve")
        .build()
        .expect("sounds");
    Fixture {
        client: bus.connect().expect("client"),
        _bus: bus,
        _server: server,
        _sounds: sounds,
        played,
    }
}

impl Fixture {
    fn notify(&self, replaces_id: u32, hints: HashMap<&str, Value<'_>>) -> u32 {
        self.client
            .call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "Notify",
                &(
                    "tests",
                    replaces_id,
                    "",
                    "summary",
                    "body",
                    Vec::<&str>::new(),
                    hints,
                    -1_i32,
                ),
            )
            .expect("Notify")
            .body()
            .deserialize()
            .expect("id")
    }

    fn heard(&self) -> Option<String> {
        self.played.recv_timeout(WAIT).ok()
    }

    fn silent(&self) -> bool {
        self.played.recv_timeout(QUIET).is_err()
    }
}

#[test]
fn a_new_notification_plays_its_sound_and_an_update_to_it_does_not() {
    let fixture = start();
    let id = fixture.notify(0, HashMap::new());
    assert_eq!(fixture.heard().as_deref(), Some("message-new-instant"));

    fixture.notify(id, HashMap::new());
    assert!(fixture.silent());

    fixture.notify(
        0,
        HashMap::from([("sound-name", Value::from("battery-low"))]),
    );
    assert_eq!(fixture.heard().as_deref(), Some("battery-low"));

    fixture.notify(0, HashMap::from([("suppress-sound", Value::from(true))]));
    assert!(fixture.silent());
}

#[test]
fn do_not_disturb_silences_all_but_critical_notifications() {
    let fixture = start();
    NotificationCenter::new(&fixture.client)
        .expect("center")
        .set_do_not_disturb(true)
        .expect("Do Not Disturb");

    fixture.notify(0, HashMap::new());
    assert!(fixture.silent());

    fixture.notify(0, HashMap::from([("urgency", Value::from(2_u8))]));
    assert_eq!(fixture.heard().as_deref(), Some("message-new-instant"));
}
