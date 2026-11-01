#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bande_test_bus::SessionBus;
use xdg_desktop_portal_bande::request::Cancellation;
use xdg_desktop_portal_bande::screenshot::{Capture, Screenshot, ScreenshotUi};
use xdg_desktop_portal_bande::{BUS_NAME, FRONTEND, OBJECT_PATH};
use zbus::blocking::Connection;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

struct Ui {
    captures: Mutex<Sender<Capture>>,
}

impl ScreenshotUi for Ui {
    fn capture(&self, capture: &Capture, _cancellation: &Cancellation) -> Option<PathBuf> {
        if let Ok(captures) = self.captures.lock() {
            let _ = captures.send(capture.clone());
        }
        Some(PathBuf::from("/home/anna/Pictures/Screenshot 1.png"))
    }

    fn pick_color(&self, _app_id: &str, _cancellation: &Cancellation) -> Option<(f64, f64, f64)> {
        None
    }
}

fn screenshot(caller: &Connection) -> (u32, HashMap<String, OwnedValue>) {
    let options = HashMap::from([("permission_store_checked", Value::from(true))]);
    caller
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some("org.freedesktop.impl.portal.Screenshot"),
            "Screenshot",
            &(
                ObjectPath::from_static_str_unchecked(
                    "/org/freedesktop/portal/desktop/request/1_7/s",
                ),
                "org.example.Paint",
                "",
                options,
            ),
        )
        .expect("Screenshot")
        .body()
        .deserialize()
        .expect("(ua{sv})")
}

#[test]
fn a_stored_screenshot_permission_is_trusted_only_from_the_desktop_portal() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let service = bus.connect().expect("portal connection");
    let (sender, captures) = mpsc::channel();
    let ui = Arc::new(Ui {
        captures: Mutex::new(sender),
    });
    service
        .object_server()
        .at(OBJECT_PATH, Screenshot { ui })
        .expect("screenshot");
    service.request_name(BUS_NAME).expect("portal name");
    let frontend = bus.connect().expect("frontend connection");
    frontend.request_name(FRONTEND).expect("frontend name");
    let app = bus.connect().expect("app connection");

    let (response, results) = screenshot(&frontend);
    assert_eq!(response, 0);
    assert_eq!(
        results
            .get("uri")
            .and_then(|uri| String::try_from(uri.try_clone().ok()?).ok()),
        Some("file:///home/anna/Pictures/Screenshot%201.png".to_owned())
    );
    let wait = Duration::from_secs(5);
    assert!(!captures.recv_timeout(wait).expect("capture").ask_permission);

    screenshot(&app);
    assert!(captures.recv_timeout(wait).expect("capture").ask_permission);
}
