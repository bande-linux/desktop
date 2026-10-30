#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use support::{Shot, shot};
use unbound_test_bus::SessionBus;
use unbound_toolkit_platform::FrameRegions;
use zbus::zvariant::{OwnedValue, Value};

const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const APPEARANCE: &str = "org.freedesktop.appearance";
const OVER_CLOSE: (i32, i32) = (24, 24);
const AWAY: (i32, i32) = (200, 120);

static DARK: AtomicBool = AtomicBool::new(false);

struct Settings;

#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl Settings {
    fn read_all(&self, _namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        let scheme = if DARK.load(Ordering::SeqCst) { 1u32 } else { 2 };
        let appearance = HashMap::from([(
            "color-scheme".to_owned(),
            OwnedValue::try_from(Value::from(scheme)).expect("owned value"),
        )]);
        HashMap::from([(APPEARANCE.to_owned(), appearance)])
    }
}

#[test]
fn the_window_frame_matches_its_goldens_in_both_schemes_at_1x_and_1_5x() {
    let bus = SessionBus::start().expect("private session bus");
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
    }
    let portal = bus.connect().expect("portal connection");
    portal
        .object_server()
        .at(PORTAL_PATH, Settings)
        .expect("serve the settings");
    portal
        .request_name("org.freedesktop.portal.Desktop")
        .expect("own the portal name");
    for dark in [false, true] {
        DARK.store(dark, Ordering::SeqCst);
        for scale in [1.0, 1.5] {
            for (state, pointer) in [("rest", AWAY), ("close-hovered", OVER_CLOSE)] {
                let scheme = if dark { "dark" } else { "light" };
                shot(
                    &format!("window-frame-{state}-{scheme}-{scale}x"),
                    |_| None,
                    Shot {
                        scale,
                        dark,
                        pointer,
                    },
                );
            }
        }
        let scheme = if dark { "dark" } else { "light" };
        shot(
            &format!("window-frame-sidebar-{scheme}-1x"),
            |window| {
                window.set_frame_regions(FrameRegions {
                    sidebar: 110.0,
                    toolbar: 0.0,
                });
                None
            },
            Shot {
                scale: 1.0,
                dark,
                pointer: AWAY,
            },
        );
    }
}
