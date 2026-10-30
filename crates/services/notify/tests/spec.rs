#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use unbound_notify::NotificationServer;
use unbound_notify::client::NotificationCenter;
use unbound_test_bus::SessionBus;
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::Value;
use zbus::{MatchRule, message};

const SIGNAL_WAIT: Duration = Duration::from_secs(5);
const QUIET_WAIT: Duration = Duration::from_millis(500);
const EXPIRED: u32 = 1;
const DISMISSED: u32 = 2;
const CLOSED_BY_CALL: u32 = 3;

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    fn get_capabilities(&self) -> zbus::Result<Vec<String>>;

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

    fn close_notification(&self, id: u32) -> zbus::Result<()>;

    fn get_server_information(&self) -> zbus::Result<(String, String, String, String)>;
}

#[derive(Debug, PartialEq)]
enum Event {
    Closed(u32, u32),
    Action(u32, String),
    Token(u32, String),
}

struct Fixture {
    _bus: SessionBus,
    server: NotificationServer,
    client: NotificationsProxyBlocking<'static>,
    center: NotificationCenter<'static>,
    events: Receiver<Event>,
}

fn start() -> Fixture {
    let bus = SessionBus::start().expect("dbus-daemon");
    let server = NotificationServer::start(&bus.connect().expect("server connection"))
        .expect("notification server");
    let connection: Connection = bus.connect().expect("client connection");
    let client = NotificationsProxyBlocking::new(&connection).expect("proxy");

    let (sender, events) = mpsc::channel();
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .interface("org.freedesktop.Notifications")
        .expect("interface")
        .build();
    let signals = MessageIterator::for_match_rule(rule, &connection, None).expect("subscribe");
    thread::spawn(move || {
        for signal in signals.flatten() {
            let header = signal.header();
            let body = signal.body();
            let event = match header.member().map(|member| member.as_str()) {
                Some("NotificationClosed") => body
                    .deserialize::<(u32, u32)>()
                    .map(|(id, reason)| Event::Closed(id, reason)),
                Some("ActivationToken") => body
                    .deserialize::<(u32, String)>()
                    .map(|(id, token)| Event::Token(id, token)),
                Some("ActionInvoked") => body
                    .deserialize::<(u32, String)>()
                    .map(|(id, key)| Event::Action(id, key)),
                _ => continue,
            };
            if let Ok(event) = event {
                let _ = sender.send(event);
            }
        }
    });

    Fixture {
        _bus: bus,
        server,
        center: NotificationCenter::new(&connection).expect("center proxy"),
        client,
        events,
    }
}

impl Fixture {
    fn notify(&self, replaces_id: u32, summary: &str, actions: &[&str], timeout: i32) -> u32 {
        self.client
            .notify(
                "tests",
                replaces_id,
                "",
                summary,
                "body",
                actions,
                HashMap::new(),
                timeout,
            )
            .expect("Notify")
    }

    fn next_event(&self) -> Event {
        self.events.recv_timeout(SIGNAL_WAIT).expect("signal")
    }

    fn assert_quiet(&self) {
        assert_eq!(self.events.recv_timeout(QUIET_WAIT).ok(), None);
    }
}

#[test]
fn server_information_reports_spec_version_1_2() {
    let fixture = start();
    let (name, _vendor, _version, spec_version) = fixture
        .client
        .get_server_information()
        .expect("GetServerInformation");
    assert!(!name.is_empty());
    assert_eq!(spec_version, "1.2");
}

#[test]
fn capabilities_include_actions_body_and_persistence() {
    let fixture = start();
    let capabilities = fixture.client.get_capabilities().expect("GetCapabilities");
    for expected in ["actions", "body", "persistence"] {
        assert!(
            capabilities.iter().any(|capability| capability == expected),
            "{expected} missing from {capabilities:?}"
        );
    }
}

#[test]
fn new_notifications_get_distinct_non_zero_ids() {
    let fixture = start();
    let first = fixture.notify(0, "first", &[], 0);
    let second = fixture.notify(0, "second", &[], 0);
    assert_ne!(first, 0);
    assert_ne!(second, 0);
    assert_ne!(first, second);
}

#[test]
fn replacing_keeps_the_id_and_does_not_close_the_original() {
    let fixture = start();
    let id = fixture.notify(0, "Downloading 10%", &[], 0);
    let replaced = fixture.notify(id, "Downloading 90%", &[], 0);

    assert_eq!(replaced, id);
    fixture.assert_quiet();
    let active = fixture.server.active();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].summary, "Downloading 90%");
}

#[test]
fn close_notification_emits_reason_3() {
    let fixture = start();
    let id = fixture.notify(0, "closing", &[], 0);
    fixture
        .client
        .close_notification(id)
        .expect("CloseNotification");
    assert_eq!(fixture.next_event(), Event::Closed(id, CLOSED_BY_CALL));
    assert!(fixture.server.active().is_empty());
}

#[test]
fn closing_an_unknown_notification_is_an_error() {
    let fixture = start();
    assert!(fixture.client.close_notification(4242).is_err());
}

#[test]
fn a_positive_timeout_expires_with_reason_1() {
    let fixture = start();
    let id = fixture.notify(0, "short-lived", &[], 50);
    assert_eq!(fixture.next_event(), Event::Closed(id, EXPIRED));
}

#[test]
fn replacing_restarts_the_expiry() {
    let fixture = start();
    let id = fixture.notify(0, "first", &[], 100);
    fixture.notify(id, "second", &[], 0);
    fixture.assert_quiet();
    assert_eq!(fixture.server.active().len(), 1);
}

#[test]
fn invoking_an_action_emits_action_invoked_then_closes() {
    let fixture = start();
    let id = fixture.notify(
        0,
        "update ready",
        &["restart", "Restart now", "later", "Later"],
        0,
    );

    assert!(fixture.server.invoke_action(id, "restart").expect("invoke"));

    assert_eq!(
        fixture.next_event(),
        Event::Action(id, "restart".to_owned())
    );
    assert_eq!(fixture.next_event(), Event::Closed(id, DISMISSED));
}

#[test]
fn a_resident_notification_stays_after_an_action() {
    let fixture = start();
    let resident = Value::from(true);
    let id = fixture
        .client
        .notify(
            "tests",
            0,
            "",
            "player",
            "",
            &["pause", "Pause"],
            HashMap::from([("resident", &resident)]),
            0,
        )
        .expect("Notify");

    assert!(fixture.server.invoke_action(id, "pause").expect("invoke"));

    assert_eq!(fixture.next_event(), Event::Action(id, "pause".to_owned()));
    fixture.assert_quiet();
    assert_eq!(fixture.server.active().len(), 1);
}

#[test]
fn dismissing_emits_reason_2() {
    let fixture = start();
    let id = fixture.notify(0, "dismiss me", &[], 0);
    assert!(fixture.server.dismiss(id).expect("dismiss"));
    assert_eq!(fixture.next_event(), Event::Closed(id, DISMISSED));
}

#[test]
fn clicking_a_notification_hands_the_activation_token_before_the_default_action() {
    let fixture = start();
    let id = fixture.notify(0, "new mail", &["default", "Open"], 0);

    assert!(
        fixture
            .center
            .activate(id, "default", "token-1")
            .expect("Activate")
    );

    assert_eq!(fixture.next_event(), Event::Token(id, "token-1".to_owned()));
    assert_eq!(
        fixture.next_event(),
        Event::Action(id, "default".to_owned())
    );
    assert_eq!(fixture.next_event(), Event::Closed(id, DISMISSED));
}
