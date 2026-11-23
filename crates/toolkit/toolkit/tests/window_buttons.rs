#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::{Duration, Instant};

use bande_test_bus::SessionBus;
use bande_test_wayland::keys::BTN_LEFT;
use bande_test_wayland::{Frames, headless};
use bande_toolkit::{App, Role, Scene, Window, WindowEvent};
use zbus::blocking::connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const BANDE_APPEARANCE: &str = "org.bande.appearance";

struct Portal;

#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl Portal {
    fn read_all(&self, _namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        let buttons = OwnedValue::try_from(Value::from("right")).expect("value");
        HashMap::from([(
            BANDE_APPEARANCE.to_owned(),
            HashMap::from([("window-buttons".to_owned(), buttons)]),
        )])
    }

    #[zbus(signal)]
    async fn setting_changed(
        emitter: &SignalEmitter<'_>,
        namespace: &str,
        key: &str,
        value: Value<'_>,
    ) -> zbus::Result<()>;
}

struct Blank {
    closing: bool,
}

impl App for Blank {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        Scene {
            width,
            height,
            background: window.appearance().palette().sheet,
            items: Vec::new(),
        }
    }

    fn event(&mut self, _window: &mut Window, event: WindowEvent) {
        self.closing |= event == WindowEvent::CloseRequested;
    }

    fn finished(&self) -> bool {
        self.closing
    }

    fn animating(&self) -> bool {
        false
    }
}

#[test]
fn window_buttons_move_to_the_left_while_the_window_is_open() {
    let bus = SessionBus::start().expect("private session bus");
    // SAFETY: this binary's only test sets this before it starts the window or the compositor, so
    // no other thread reads the environment yet.
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
    }
    let portal = connection::Builder::address(bus.address())
        .expect("address")
        .name("org.freedesktop.portal.Desktop")
        .expect("portal name")
        .serve_at(PORTAL_PATH, Portal)
        .expect("portal object")
        .build()
        .expect("portal connection");
    let mut server = headless(640, 480);
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let app = thread::spawn(move || {
        let mut window = Window::open_on(
            client_end,
            Role::toplevel("Buttons", "bande-toolkit-test"),
            (240, 120),
        )
        .expect("window");
        let mut blank = Blank { closing: false };
        bande_toolkit::run(&mut window, &mut blank).expect("run the window");
    });
    let mut frames = Frames::start();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !server
        .windows()
        .iter()
        .any(|window| window.geometry().is_some())
    {
        assert!(Instant::now() < deadline, "the window never showed");
        let mut stepped = false;
        frames.run_until(&mut server, || std::mem::replace(&mut stepped, true));
    }

    let emitter = SignalEmitter::new(portal.inner(), PORTAL_PATH).expect("emitter");
    zbus::block_on(Portal::setting_changed(
        &emitter,
        BANDE_APPEARANCE,
        "window-buttons",
        Value::from("left"),
    ))
    .expect("announce the change");

    let deadline = Instant::now() + Duration::from_secs(10);
    while !app.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the top-left corner never closed the window"
        );
        if let Some(geometry) = server.windows().iter().find_map(|window| window.geometry()) {
            let close = (
                f64::from(geometry.loc.x) + 16.0,
                f64::from(geometry.loc.y) + 24.0,
            );
            server.move_pointer_to(close.into(), 0);
            server.press_button(BTN_LEFT, true, 0);
            server.press_button(BTN_LEFT, false, 0);
        }
        let pause = Instant::now() + Duration::from_millis(700);
        frames.run_until(&mut server, || Instant::now() > pause || app.is_finished());
    }
    app.join().expect("app thread");
}
