#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::time::{Duration, Instant};

use accesskit::Role;
use bande_notify::client::{Request, notify};
use bande_notify::{Banners, NotificationServer, Urgency};
use bande_settings::{Command, Feed, PaneId, SettingsView, feed, follow_app_notifications};
use bande_test_bus::SessionBus;
use bande_toolkit::{Palette, Scheme, TextSystem};
use support::{WAIT, click, find, nodes};

fn app_summary(view: &mut SettingsView) -> Option<String> {
    nodes(view)
        .into_iter()
        .find(|(_, node)| node.role() == Role::Heading && node.label() == Some("Mail"))
        .and_then(|(_, node)| node.value().map(str::to_owned))
}

fn pump(
    view: &mut SettingsView,
    feed: &Feed,
    sender: &std::sync::mpsc::Sender<bande_settings::NotificationCommand>,
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
    let server = NotificationServer::start(&connection, &bus.connect().expect("center"))
        .expect("notification server");
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
        bande_settings::SettingId::DoNotDisturb,
        bande_settings::Value::Flag(false),
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
    assert!(find(&mut view, Role::Button, "‹ Notifications").is_some());
    assert!(find(&mut view, Role::Switch, "Do Not Disturb").is_none());
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
        !server.apps()[0].rule.in_center
            && app_summary(view).as_deref() == Some("Persistent banners")
    });
    let allow = find(&mut view, Role::Switch, "Allow notifications").expect("allow");
    click(&mut view, allow);
    pump(&mut view, &feed, &sender, |view| {
        !server.apps()[0].rule.allow && app_summary(view).as_deref() == Some("Off")
    });
    assert!(find(&mut view, Role::RadioButton, "Persistent").is_none());

    let back = find(&mut view, Role::Button, "‹ Notifications").expect("back");
    click(&mut view, back);
    assert_eq!(view.selected_pane(), PaneId::Notifications);
    assert_eq!(view.focused_item().as_deref(), Some("app:Mail"));
    assert!(find(&mut view, Role::Switch, "Do Not Disturb").is_some());
}
