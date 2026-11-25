#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::time::{Duration, Instant};

use bande_compositor::{Compositor, SessionRole};
use bande_test_bus::SessionBus;
use bande_test_bus::atspi::{FakeRegistry, ObjectRef, accessible, find_named};
use bande_test_wayland::keys::{KEY_ESC, KEY_LEFTMETA};
use bande_test_wayland::{Frames, TestClient, chord, headless, roundtrip};
use zbus::blocking::{Connection, connection};

const PATIENCE: Duration = Duration::from_secs(60);
const SETTLED: Duration = Duration::from_millis(400);
const WINDOW_COORDINATES: u32 = 1;
const KEY_H: u32 = 35;
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const READER: &str = "Example Reader";
const EDITOR: &str = "Example Editor";

struct Scene {
    server: Compositor,
    frames: Frames,
    atspi: Connection,
    root: ObjectRef,
}

impl Scene {
    fn pass(&mut self, time: Duration) {
        let until = Instant::now() + time;
        self.frames
            .run_until(&mut self.server, || Instant::now() >= until);
    }

    fn press_button_of(&mut self, app: &str, button: u32) {
        let deadline = Instant::now() + PATIENCE;
        let found = loop {
            match find_named(&self.atspi, &self.root, app) {
                Some(found) => break found,
                None => assert!(Instant::now() < deadline, "{app} is not in the taskbar"),
            }
            self.pass(SETTLED);
        };
        let (x, y, width, height): (i32, i32, i32, i32) =
            accessible(&self.atspi, &found, "org.a11y.atspi.Component")
                .and_then(|component| component.call("GetExtents", &(WINDOW_COORDINATES,)))
                .expect("where the taskbar button is");
        let middle = (f64::from(x + width / 2), f64::from(y + height / 2));
        self.server.move_pointer_to(middle.into(), 0);
        self.pass(SETTLED);
        self.server.press_button(button, true, 0);
        self.pass(Duration::from_millis(100));
        self.server.press_button(button, false, 0);
        self.pass(SETTLED);
    }

    fn minimized(&self, app: &str) -> bool {
        let windows = self.server.windows();
        windows
            .iter()
            .find(|window| window.app_id() == app)
            .expect("the app's window")
            .is_minimized()
    }
}

#[test]
fn a_taskbar_press_brings_forward_only_its_own_app_and_never_holds_the_keyboard() {
    let bus = SessionBus::start().expect("private session and AT-SPI bus");
    // SAFETY: this binary's only test sets these before it starts the registry or the shell, so
    // no other thread reads the environment yet.
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
        std::env::set_var("AT_SPI_BUS_ADDRESS", bus.address());
    }
    let registry = FakeRegistry::start(&bus).expect("AT-SPI registry");
    let mut server = headless(1024, 768);
    server
        .start_session_client(
            Path::new(env!("CARGO_BIN_EXE_bande-shell")),
            SessionRole::Shell,
        )
        .expect("start the shell");
    let mut editor = TestClient::window(&mut server, EDITOR);
    let mut reader = TestClient::window(&mut server, READER);
    let mut frames = Frames::start();
    let mut root = None;
    frames.run_until(&mut server, || {
        root = registry.embedded_root(Duration::ZERO);
        root.is_some()
    });
    let mut scene = Scene {
        server,
        frames,
        atspi: connection::Builder::address(bus.address())
            .expect("address")
            .method_timeout(Duration::from_secs(2))
            .build()
            .expect("screen reader connection"),
        root: root.expect("the shell's accessible root"),
    };

    scene.press_button_of(READER, BTN_LEFT);
    chord(&mut scene.server, &[KEY_LEFTMETA, KEY_H]);
    scene.pass(SETTLED);
    assert!(scene.minimized(READER));

    scene.press_button_of(EDITOR, BTN_LEFT);
    assert!(scene.minimized(READER), "another app's window came back");
    roundtrip(&mut scene.server, &mut [&mut editor, &mut reader]);
    assert!(editor.keyboard_focus_on(0));

    scene.press_button_of(READER, BTN_RIGHT);
    roundtrip(&mut scene.server, &mut [&mut editor, &mut reader]);
    assert!(!editor.keyboard_focus_on(0), "the menu has the keyboard");
    chord(&mut scene.server, &[KEY_ESC]);
    scene.pass(SETTLED);
    roundtrip(&mut scene.server, &mut [&mut editor, &mut reader]);
    assert!(
        editor.keyboard_focus_on(0),
        "the menu returns the keyboard to the window it was taken from"
    );
    assert!(scene.minimized(READER));

    scene.press_button_of(READER, BTN_LEFT);
    assert!(!scene.minimized(READER));
}
