#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

use bande_compositor::{Compositor, FrameClock, FrameScheduler, REFRESH, Renderer};
use bande_graphics::{Accent, Appearance, Color, Contrast, Scene, Scheme, TextSize};
use bande_test_bus::SessionBus;
use bande_toolkit_platform::{App, Role, Window, WindowEvent};
use smithay::utils::Size;
use zbus::zvariant::{OwnedValue, Value};

const OUTPUT: (i32, i32) = (320, 240);
const TIMEOUT: Duration = Duration::from_secs(60);
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const APPEARANCE: &str = "org.freedesktop.appearance";
const BANDE_APPEARANCE: &str = "org.bande.appearance";
const TINT: [f64; 3] = [0.2, 0.4, 0.6];
const ACCENT: (f64, f64, f64) = (0.8, 0.2, 0.4);

struct Settings;

#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl Settings {
    fn read_all(&self, namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        let owned = |value: Value<'_>| OwnedValue::try_from(value).expect("owned value");
        let appearance = HashMap::from([
            ("color-scheme".to_owned(), owned(Value::from(1u32))),
            ("accent-color".to_owned(), owned(Value::from(ACCENT))),
        ]);
        let bande = HashMap::from([
            ("frame-tint".to_owned(), owned(Value::from(TINT.to_vec()))),
            ("text-size".to_owned(), owned(Value::from(150u32))),
        ]);
        HashMap::from([
            (APPEARANCE.to_owned(), appearance),
            (BANDE_APPEARANCE.to_owned(), bande),
        ])
        .into_iter()
        .filter(|(namespace, _)| namespaces.is_empty() || namespaces.contains(namespace))
        .collect()
    }
}

struct Follower {
    seen: Vec<(Appearance, Appearance)>,
    report: Sender<usize>,
}

impl App for Follower {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        Scene {
            width,
            height,
            background: window.appearance().palette().sheet,
            items: Vec::new(),
        }
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        if let WindowEvent::AppearanceChanged(appearance) = event {
            self.seen.push((appearance, window.appearance()));
            let _ = self.report.send(self.seen.len());
        }
    }

    fn finished(&self) -> bool {
        self.seen.len() == 3
    }
}

#[test]
fn the_window_follows_the_portal_colour_scheme_accent_contrast_motion_and_text_size() {
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
    let change = |namespace: &str, key: &str, value: Value<'_>| {
        portal
            .emit_signal(
                None::<()>,
                PORTAL_PATH,
                SETTINGS,
                "SettingChanged",
                &(namespace, key, value),
            )
            .expect("emit SettingChanged");
    };

    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let (report, reports) = mpsc::channel();
    let app = thread::spawn(move || {
        let mut window = Window::open_on(
            client_end,
            Role::toplevel("Appearance", "bande-toolkit-test"),
            (120, 80),
        )
        .expect("window");
        let mut follower = Follower {
            seen: Vec::new(),
            report,
        };
        bande_toolkit_platform::run(&mut window, &mut follower).expect("run the window");
        follower.seen
    });

    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut scheduler = FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH));
    let start = Instant::now();
    while !app.is_finished() {
        assert!(start.elapsed() < TIMEOUT, "the window stopped following");
        server.dispatch().expect("server dispatch");
        scheduler
            .poll(|| start.elapsed(), &mut server, &mut renderer)
            .expect("compositor frame");
        if let Ok(seen) = reports.try_recv() {
            match seen {
                1 => {
                    change(
                        "org.gnome.desktop.interface",
                        "color-scheme",
                        Value::from("prefer-light"),
                    );
                    change(APPEARANCE, "contrast", Value::from(1u32));
                    change(APPEARANCE, "accent-color", Value::from((-1.0, -1.0, -1.0)));
                    change(APPEARANCE, "reduced-motion", Value::from(1u32));
                    change(
                        BANDE_APPEARANCE,
                        "frame-tint",
                        Value::from(Vec::<f64>::new()),
                    );
                    change(BANDE_APPEARANCE, "text-size", Value::from(300u32));
                }
                2 => change(APPEARANCE, "color-scheme", Value::from(0u32)),
                _ => {}
            }
        }
        thread::sleep(Duration::from_millis(1));
    }

    let dark = Appearance {
        scheme: Scheme::Dark,
        accent: Accent::Rose,
        frame_tint: Some(Color::hex(0x336699)),
        text_size: TextSize::from_percent(150),
        ..Appearance::default()
    };
    let dark_reduced = Appearance {
        scheme: Scheme::Dark,
        contrast: Contrast::Increased,
        reduce_motion: true,
        text_size: TextSize::LARGEST,
        ..Appearance::default()
    };
    let light = Appearance {
        contrast: Contrast::Increased,
        reduce_motion: true,
        text_size: TextSize::LARGEST,
        ..Appearance::default()
    };
    assert_eq!(
        app.join().expect("app thread"),
        [(dark, dark), (dark_reduced, dark_reduced), (light, light)]
    );
}
