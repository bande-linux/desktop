#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use bande_test_bus::SessionBus;
use bande_tray::{BUS_NAME, OBJECT_PATH, Watcher};
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::OwnedValue;
use zbus::{MatchRule, message};

const INTERFACE: &str = "org.kde.StatusNotifierWatcher";
const WAIT: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(300);

struct Fixture {
    _watcher: Watcher,
    observer: Connection,
    signals: Receiver<(String, Option<String>)>,
    bus: SessionBus,
}

fn start() -> Fixture {
    let bus = SessionBus::start().expect("dbus-daemon");
    let watcher = Watcher::start(&bus.connect().expect("watcher connection")).expect("watcher");
    let observer = bus.connect().expect("observer");
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .interface(INTERFACE)
        .expect("interface")
        .build();
    let iterator = MessageIterator::for_match_rule(rule, &observer, None).expect("subscribe");
    let (sender, signals) = mpsc::channel();
    std::thread::spawn(move || {
        for signal in iterator.flatten() {
            let member = signal
                .header()
                .member()
                .map(|member| member.to_string())
                .unwrap_or_default();
            let argument = signal.body().deserialize::<String>().ok();
            let _ = sender.send((member, argument));
        }
    });
    Fixture {
        _watcher: watcher,
        observer,
        signals,
        bus,
    }
}

fn register(connection: &Connection, method: &str, service: &str) {
    connection
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some(INTERFACE),
            method,
            &service,
        )
        .expect(method);
}

impl Fixture {
    fn property(&self, name: &str) -> OwnedValue {
        self.observer
            .call_method(
                Some(BUS_NAME),
                OBJECT_PATH,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(INTERFACE, name),
            )
            .expect("Get")
            .body()
            .deserialize()
            .expect("v")
    }

    fn items(&self) -> Vec<String> {
        Vec::try_from(self.property("RegisteredStatusNotifierItems")).expect("as")
    }

    fn next(&self) -> (String, Option<String>) {
        self.signals.recv_timeout(WAIT).expect("signal")
    }
}

#[test]
fn an_item_registered_by_bus_name_uses_the_default_path() {
    let fixture = start();
    let app = fixture.bus.connect().expect("app");
    app.request_name("org.kde.StatusNotifierItem-4077-1")
        .expect("name");
    register(
        &app,
        "RegisterStatusNotifierItem",
        "org.kde.StatusNotifierItem-4077-1",
    );

    let id = "org.kde.StatusNotifierItem-4077-1/StatusNotifierItem".to_owned();
    assert_eq!(
        fixture.next(),
        ("StatusNotifierItemRegistered".to_owned(), Some(id.clone()))
    );
    assert_eq!(fixture.items(), [id]);
}

#[test]
fn an_item_registered_by_object_path_belongs_to_its_caller() {
    let fixture = start();
    let app = fixture.bus.connect().expect("app");
    register(
        &app,
        "RegisterStatusNotifierItem",
        "/org/ayatana/NotificationItem/nextcloud",
    );

    let unique = app.unique_name().expect("unique name").to_string();
    assert_eq!(
        fixture.items(),
        [format!("{unique}/org/ayatana/NotificationItem/nextcloud")]
    );
}

#[test]
fn registering_twice_lists_the_item_once() {
    let fixture = start();
    let app = fixture.bus.connect().expect("app");
    register(&app, "RegisterStatusNotifierItem", "/tray");
    register(&app, "RegisterStatusNotifierItem", "/tray");
    fixture.next();
    assert_eq!(fixture.items().len(), 1);
    assert!(fixture.signals.recv_timeout(QUIET).is_err());
}

#[test]
fn an_item_is_unregistered_when_its_application_leaves() {
    let fixture = start();
    let app = fixture.bus.connect().expect("app");
    register(&app, "RegisterStatusNotifierItem", "/tray");
    let (_, id) = fixture.next();

    drop(app);
    assert_eq!(
        fixture.next(),
        ("StatusNotifierItemUnregistered".to_owned(), id)
    );
    assert!(fixture.items().is_empty());
}

#[test]
fn hosts_are_tracked_until_the_last_one_leaves() {
    let fixture = start();
    assert!(!bool::try_from(fixture.property("IsStatusNotifierHostRegistered")).expect("b"));
    let panel = fixture.bus.connect().expect("panel");
    register(
        &panel,
        "RegisterStatusNotifierHost",
        "org.kde.StatusNotifierHost-1",
    );
    assert_eq!(fixture.next().0, "StatusNotifierHostRegistered");
    assert!(bool::try_from(fixture.property("IsStatusNotifierHostRegistered")).expect("b"));

    drop(panel);
    assert_eq!(fixture.next().0, "StatusNotifierHostUnregistered");
    assert!(!bool::try_from(fixture.property("IsStatusNotifierHostRegistered")).expect("b"));
}

#[test]
fn the_protocol_version_is_0() {
    let fixture = start();
    assert_eq!(
        i32::try_from(fixture.property("ProtocolVersion")).expect("i"),
        0
    );
}
