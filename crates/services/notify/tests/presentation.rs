#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use unbound_notify::client::NotificationCenter;
use unbound_notify::{NotificationServer, Presentation, Urgency};
use unbound_test_bus::SessionBus;
use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;
use zbus::names::InterfaceName;
use zbus::zvariant::{OwnedValue, Value};

const WAIT: Duration = Duration::from_secs(5);
const CENTER: &str = "org.unbound.NotificationCenter";

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, &Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;
}

struct Fixture {
    _bus: SessionBus,
    _server: NotificationServer,
    connection: Connection,
    client: NotificationsProxyBlocking<'static>,
    center: NotificationCenter<'static>,
    presented: Receiver<Presentation>,
}

fn start() -> Fixture {
    let bus = SessionBus::start().expect("dbus-daemon");
    let server = NotificationServer::start(&bus.connect().expect("server connection"))
        .expect("notification server");
    let connection = bus.connect().expect("client connection");
    let client = NotificationsProxyBlocking::new(&connection).expect("proxy");
    let center = NotificationCenter::new(&connection).expect("center proxy");
    let changes = center
        .receive_presentation_changed()
        .expect("subscribe to the presentation");
    let (sender, presented) = mpsc::channel();
    thread::spawn(move || {
        for change in changes {
            if let Ok(arguments) = change.args() {
                let _ = sender.send(Presentation::from_wire(arguments.presentation));
            }
        }
    });
    Fixture {
        _bus: bus,
        _server: server,
        connection,
        client,
        center,
        presented,
    }
}

impl Fixture {
    fn notify(&self, summary: &str, hints: HashMap<&str, &Value<'_>>) -> u32 {
        self.client
            .notify("Mail", 0, "", summary, "", &[], hints, -1)
            .expect("Notify")
    }

    fn next(&self) -> Presentation {
        self.presented.recv_timeout(WAIT).expect("presentation")
    }

    fn center_properties(&self) -> PropertiesProxy<'static> {
        center_properties_on(&self.connection)
    }
}

#[test]
fn hints_reach_the_presentation() {
    let fixture = start();
    let critical = Value::from(2u8);
    let entry = Value::from("org.example.Mail");
    let id = fixture.notify(
        "Disk full",
        HashMap::from([("urgency", &critical), ("desktop-entry", &entry)]),
    );

    let presentation = fixture.next();
    assert_eq!(presentation.banners.len(), 1);
    let stack = &presentation.banners[0];
    assert_eq!(stack.group, "org.example.Mail");
    assert_eq!(stack.notifications[0].id, id);
    assert_eq!(stack.notifications[0].urgency, Urgency::Critical);
}

#[test]
fn a_transient_notification_is_kept_out_of_the_center() {
    let fixture = start();
    let transient = Value::from(true);
    fixture.notify("Volume 40%", HashMap::from([("transient", &transient)]));

    let presentation = fixture.next();
    assert_eq!(presentation.banners.len(), 1);
    assert!(presentation.center.is_empty());
}

#[test]
fn do_not_disturb_is_a_writable_property() {
    let fixture = start();
    let properties = fixture.center_properties();
    let interface = InterfaceName::from_static_str_unchecked(CENTER);
    assert_eq!(
        properties
            .get(interface.clone(), "DoNotDisturb")
            .expect("Get"),
        OwnedValue::from(false)
    );

    properties
        .set(interface.clone(), "DoNotDisturb", Value::from(true))
        .expect("Set");
    assert!(fixture.next().do_not_disturb);
    assert_eq!(
        properties.get(interface, "DoNotDisturb").expect("Get"),
        OwnedValue::from(true)
    );

    fixture.notify("New message", HashMap::new());
    let presentation = fixture.next();
    assert!(presentation.banners.is_empty());
    assert_eq!(presentation.center.len(), 1);
}

#[test]
fn the_screen_can_fetch_what_to_show_and_swipe_a_banner_into_the_center() {
    let fixture = start();
    let id = fixture.notify("New message", HashMap::new());
    fixture.next();

    let shown = Presentation::from_wire(fixture.center.get_presentation().expect("fetch"));
    assert_eq!(shown.banners[0].notifications[0].id, id);

    fixture.center.hide_banner(id).expect("HideBanner");
    let presentation = fixture.next();
    assert!(presentation.banners.is_empty());
    assert_eq!(presentation.center[0].notifications[0].id, id);

    fixture.center.clear_all().expect("ClearAll");
    assert!(fixture.next().center.is_empty());
}

#[test]
fn clicking_a_notification_without_a_default_action_removes_it() {
    let fixture = start();
    let id = fixture.notify("Download finished", HashMap::new());
    fixture.next();

    assert!(
        !fixture
            .center
            .activate(id, "default", "token")
            .expect("Activate")
    );
    let presentation = fixture.next();
    assert!(presentation.banners.is_empty());
    assert!(presentation.center.is_empty());
}

fn center_properties_on(connection: &Connection) -> PropertiesProxy<'static> {
    PropertiesProxy::builder(connection)
        .destination(unbound_notify::BUS_NAME)
        .expect("destination")
        .path(unbound_notify::OBJECT_PATH)
        .expect("path")
        .build()
        .expect("properties proxy")
}

#[test]
fn do_not_disturb_is_remembered_across_restarts() {
    let state = tempfile::tempdir().expect("state directory");
    let file = state.path().join("unbound").join("do-not-disturb");
    let interface = InterfaceName::from_static_str_unchecked(CENTER);

    let first_session = SessionBus::start().expect("dbus-daemon");
    let _first = NotificationServer::start_remembering(
        &first_session.connect().expect("server connection"),
        file.clone(),
    )
    .expect("notification server");
    let first_client = first_session.connect().expect("client connection");
    center_properties_on(&first_client)
        .set(interface.clone(), "DoNotDisturb", Value::from(true))
        .expect("Set");

    let second_session = SessionBus::start().expect("dbus-daemon");
    let _second = NotificationServer::start_remembering(
        &second_session.connect().expect("server connection"),
        file.clone(),
    )
    .expect("restarted notification server");
    let second_client = second_session.connect().expect("client connection");
    let properties = center_properties_on(&second_client);
    assert_eq!(
        properties
            .get(interface.clone(), "DoNotDisturb")
            .expect("Get"),
        OwnedValue::from(true),
        "Do Not Disturb is still on after the restart"
    );

    properties
        .set(interface, "DoNotDisturb", Value::from(false))
        .expect("Set");
    assert!(!file.exists(), "turning it off forgets it");
}
