mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use common::{Options, Portal, get};
use xdg_desktop_portal_unbound::OBJECT_PATH;
use xdg_desktop_portal_unbound::request::Cancellation;
use xdg_desktop_portal_unbound::screenshot::{Capture, Screenshot, ScreenshotUi};
use zbus::zvariant::{ObjectPath, Value};

const INTERFACE: &str = "org.freedesktop.impl.portal.Screenshot";
const WAIT: Duration = Duration::from_secs(5);

struct FakeUi {
    captures: Mutex<Sender<Capture>>,
    shot: Option<PathBuf>,
}

impl ScreenshotUi for FakeUi {
    fn capture(&self, capture: &Capture, _cancellation: &Cancellation) -> Option<PathBuf> {
        if let Ok(captures) = self.captures.lock() {
            let _ = captures.send(capture.clone());
        }
        self.shot.clone()
    }

    fn pick_color(&self, _app_id: &str, _cancellation: &Cancellation) -> Option<(f64, f64, f64)> {
        Some((1.0, 0.5, 0.0))
    }
}

fn start(shot: Option<&str>) -> (Portal, Receiver<Capture>) {
    let (sender, captures) = mpsc::channel();
    let ui = Arc::new(FakeUi {
        captures: Mutex::new(sender),
        shot: shot.map(PathBuf::from),
    });
    let portal = Portal::start(|connection| {
        connection
            .object_server()
            .at(OBJECT_PATH, Screenshot { ui })
            .expect("screenshot");
    });
    (portal, captures)
}

fn handle() -> ObjectPath<'static> {
    ObjectPath::from_static_str_unchecked("/org/freedesktop/portal/desktop/request/1_7/s")
}

#[test]
fn a_screenshot_returns_the_file_uri() {
    let (portal, captures) = start(Some("/home/anna/Pictures/Screenshot 1.png"));
    let options = Options::from([
        ("interactive", Value::from(true)),
        ("permission_store_checked", Value::from(true)),
    ]);
    let (response, results) = portal.request(
        INTERFACE,
        "Screenshot",
        &(handle(), "org.example.Paint", "", options),
    );
    assert_eq!(response, 0);
    assert_eq!(
        get::<String>(&results, "uri"),
        "file:///home/anna/Pictures/Screenshot%201.png"
    );
    let capture = captures.recv_timeout(WAIT).expect("capture");
    assert!(capture.interactive);
    assert!(!capture.ask_permission);
}

#[test]
fn without_a_stored_permission_the_user_is_asked() {
    let (portal, captures) = start(Some("/tmp/s.png"));
    portal.request(
        INTERFACE,
        "Screenshot",
        &(handle(), "org.example.Paint", "", Options::new()),
    );
    let capture = captures.recv_timeout(WAIT).expect("capture");
    assert!(capture.ask_permission);
    assert!(!capture.interactive);
}

#[test]
fn a_refused_screenshot_is_cancelled() {
    let (portal, _) = start(None);
    let (response, results) = portal.request(
        INTERFACE,
        "Screenshot",
        &(handle(), "org.example.Paint", "", Options::new()),
    );
    assert_eq!(response, 1);
    assert!(results.is_empty());
}

#[test]
fn pick_color_returns_rgb_fractions() {
    let (portal, _) = start(None);
    let (response, results) = portal.request(
        INTERFACE,
        "PickColor",
        &(handle(), "org.example.Paint", "", Options::new()),
    );
    assert_eq!(response, 0);
    assert_eq!(get::<(f64, f64, f64)>(&results, "color"), (1.0, 0.5, 0.0));
}
