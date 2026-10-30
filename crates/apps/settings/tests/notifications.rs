#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::time::{Duration, Instant};

use accesskit::Role;
use support::{WAIT, click, find};
use unbound_notify::client::{Request, notify};
use unbound_notify::{Banners, NotificationServer, Urgency};
use unbound_settings::{Command, Feed, PaneId, SettingsView, feed, follow_app_notifications};
use unbound_test_bus::SessionBus;
use unbound_toolkit::{Palette, Scheme, TextSystem};

fn pump(
    view: &mut SettingsView,
    feed: &Feed,
    sender: &std::sync::mpsc::Sender<unbound_settings::NotificationCommand>,
    done: impl Fn(&mut SettingsView) -> bool,
) {
    let deadline = Instant::now() + WAIT;
    loop {
        for update in feed.drain() {
            view.apply(update);
        }
        for command in view.take_commands() {
            if let Command::Notifications(command) = command {
                sender.send(command).expect("notifications worker");
            }
        }
        if done(view) {
            return;
        }
        assert!(Instant::now() < deadline, "the view never got there");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn each_app_that_notified_gets_its_own_rules() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let connection = bus.connect().expect("connection");
    let server = NotificationServer::start(&connection).expect("notification server");
    let client = bus.connect().expect("client");
    notify(
        &client,
        &Request {
            app_name: "Mail",
            replaces_id: 0,
            app_icon: "",
            summary: "New message",
            body: "",
            actions: &[],
            urgency: Urgency::Normal,
            sound: None,
        },
    )
    .expect("notify");
    let (feed, updates) = feed().expect("feed");
    let sender = follow_app_notifications(&client, updates);
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.set_value(
        unbound_settings::SettingId::DoNotDisturb,
        unbound_settings::Value::Flag(false),
    );
    view.select_pane(PaneId::Notifications);
    pump(&mut view, &feed, &sender, |view| {
        find(
            view,
            Role::Button,
            "Mail, Temporary banners, Notification Center",
        )
        .is_some()
    });
    let mail = find(&mut view, Role::Button, "Mail").expect("Mail row");
    click(&mut view, mail);
    let persistent = find(&mut view, Role::RadioButton, "Persistent").expect("Persistent");
    click(&mut view, persistent);
    pump(&mut view, &feed, &sender, |_| {
        server
            .apps()
            .iter()
            .any(|app| app.rule.banners == Banners::Persistent)
    });
    let center = find(&mut view, Role::Switch, "Show in Notification Center").expect("switch");
    click(&mut view, center);
    pump(&mut view, &feed, &sender, |view| {
        !server.apps()[0].rule.in_center && find(view, Role::Button, "Mail, Persistent").is_some()
    });
    let allow = find(&mut view, Role::Switch, "Allow notifications").expect("allow");
    click(&mut view, allow);
    pump(&mut view, &feed, &sender, |view| {
        !server.apps()[0].rule.allow && find(view, Role::Button, "Mail, Off").is_some()
    });
    assert!(find(&mut view, Role::RadioButton, "Persistent").is_none());
}
