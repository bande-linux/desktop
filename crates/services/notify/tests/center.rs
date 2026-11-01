#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use bande_notify::client::NotificationCenter;
use bande_notify::{BUS_NAME, NotificationServer, OBJECT_PATH, Presentation};
use bande_test_bus::SessionBus;
use zbus::zvariant::Value;

#[test]
fn a_notification_shows_as_a_banner_and_only_the_center_name_can_read_or_dismiss_it() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let _server = NotificationServer::start(
        &bus.connect().expect("server connection"),
        &bus.connect().expect("center connection"),
    )
    .expect("notification server");
    let app = bus.connect().expect("app connection");
    let shell = bus.connect().expect("shell connection");
    let center = NotificationCenter::new(&shell).expect("center proxy");
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
    let next = || {
        presented
            .recv_timeout(Duration::from_secs(5))
            .expect("presentation")
    };

    let id: u32 = app
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "Mail",
                0u32,
                "",
                "Your code is 123456",
                "",
                Vec::<&str>::new(),
                HashMap::<&str, Value>::new(),
                -1i32,
            ),
        )
        .expect("Notify")
        .body()
        .deserialize()
        .expect("id");
    assert_eq!(next().banners[0].notifications[0].id, id);

    for method in ["GetPresentation", "ClearAll"] {
        let refused = app.call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some("org.bande.NotificationCenter"),
            method,
            &(),
        );
        assert!(refused.is_err(), "{method} through the notification name");
    }

    center.hide_banner(id).expect("HideBanner");
    let presentation = next();
    assert!(presentation.banners.is_empty());
    assert_eq!(presentation.center[0].notifications[0].id, id);
    center.clear_all().expect("ClearAll");
    assert!(next().center.is_empty());
}
