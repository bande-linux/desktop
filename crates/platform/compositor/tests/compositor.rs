#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::thread;
use std::time::{Duration, Instant};

use bande_compositor::{
    Compositor, KEYBOARD_MONITOR_NAME, SCREEN_READER_NAME, UNRESPONSIVE_AFTER,
    serve_keyboard_monitor,
};
use bande_test_bus::SessionBus;
use bande_test_wayland::{
    BLUE, ConstraintAdjustment, PopupRequest, RED, SeparateApp, TestClient, act_as_separate_app,
    chord, headless, roundtrip,
};

const KEY_ESC: u32 = 1;
const KEY_A: u32 = 30;
const KEY_ENTER: u32 = 28;
const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTALT: u32 = 56;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_LEFTMETA: u32 = 125;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_DOWN: u32 = 108;
const BTN_LEFT: u32 = 0x110;
const MONITOR: &str = "org.freedesktop.a11y.KeyboardMonitor";
const MONITOR_PATH: &str = "/org/freedesktop/a11y/Manager";

fn focused_app() -> (Compositor, TestClient) {
    let mut server = headless(560, 520);
    let mut editor = TestClient::window(&mut server, "org.example.Editor");
    server.move_pointer_to((280.0, 260.0).into(), 0);
    server.press_button(BTN_LEFT, true, 0);
    server.press_button(BTN_LEFT, false, 0);
    roundtrip(&mut server, &mut [&mut editor]);
    assert!(editor.keyboard_focus_on(0));
    (server, editor)
}

fn locked(server: &mut Compositor) -> TestClient {
    let mut lock = TestClient::connect_as_shell(server);
    lock.use_seat(server);
    lock.lock_session(server);
    lock.show_lock_surface(server, BLUE);
    lock
}

#[test]
fn while_locked_no_popup_grab_or_input_method_takes_keys_from_the_lock_surface() {
    let (mut server, mut editor) = focused_app();
    let mut input_method = TestClient::connect(&mut server);
    input_method.use_seat(&mut server);
    input_method.become_input_method(&mut server);
    input_method.grab_keyboard(&mut server);
    let mut lock = locked(&mut server);
    roundtrip(
        &mut server,
        &mut [&mut lock, &mut editor, &mut input_method],
    );
    assert!(lock.lock_surface_has_keyboard());
    let popup = editor.open_popup(
        &mut server,
        &PopupRequest {
            parent: 0,
            anchor_rect: (0, 0, 1, 1),
            size: (20, 20),
            adjustment: ConstraintAdjustment::empty(),
            grab: true,
            color: RED,
        },
    );
    let clicks_before = editor.buttons().len();

    chord(&mut server, &[KEY_A]);
    server.move_pointer_to((280.0, 260.0).into(), 3);
    server.press_button(BTN_LEFT, true, 4);
    server.press_button(BTN_LEFT, false, 5);
    roundtrip(
        &mut server,
        &mut [&mut lock, &mut editor, &mut input_method],
    );

    assert!(editor.popup_dismissed(popup));
    assert!(editor.keys().is_empty());
    assert_eq!(editor.buttons().len(), clicks_before);
    assert!(input_method.grabbed_keys().is_empty());
    assert!(lock.lock_surface_has_keyboard());
    assert!(lock.keys().contains(&(KEY_A, true)));
}

#[test]
fn only_the_screen_reader_may_grab_the_keyboard_and_it_lets_go_when_it_leaves() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let (mut server, mut app) = focused_app();
    let handle = serve_keyboard_monitor(&bus.connect().expect("connection")).expect("monitor");
    server.monitor_keyboard(handle);
    let grab = |connection: &zbus::blocking::Connection| {
        connection.call_method(
            Some(KEYBOARD_MONITOR_NAME),
            MONITOR_PATH,
            Some(MONITOR),
            "GrabKeyboard",
            &(),
        )
    };
    let stranger = bus.connect().expect("other connection");
    assert!(grab(&stranger).is_err());

    let reader = bus.connect().expect("screen reader connection");
    reader
        .request_name(SCREEN_READER_NAME)
        .expect("screen reader name");
    grab(&reader).expect("the screen reader grabs the keyboard");
    chord(&mut server, &[KEY_A]);
    roundtrip(&mut server, &mut [&mut app]);
    assert!(app.keys().is_empty());

    drop(reader);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.keys().is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
        chord(&mut server, &[KEY_A]);
        roundtrip(&mut server, &mut [&mut app]);
    }
    assert!(!app.keys().is_empty());
}

#[test]
fn a_hung_app_is_force_quit_from_the_keyboard_while_the_shell_is_hung() {
    if act_as_separate_app() {
        return;
    }
    let mut server = headless(560, 520);
    let mut bar = TestClient::connect_as_shell(&mut server);
    bar.join_shell(&mut server);
    let mut stuck = SeparateApp::launch(
        &mut server,
        "a_hung_app_is_force_quit_from_the_keyboard_while_the_shell_is_hung",
    );
    stuck.show_window_of(&mut server, 64, "org.example.Stuck");
    let mut editor = TestClient::window(&mut server, "org.example.Editor");
    editor.inhibit_shortcuts(&mut server, 0);
    roundtrip(&mut server, &mut [&mut editor]);

    chord(&mut server, &[KEY_LEFTCTRL, KEY_LEFTALT, KEY_ESC]);
    let start = Duration::from_secs(10);
    server.watchdog_tick(start);
    roundtrip(&mut server, &mut [&mut editor]);
    server.watchdog_tick(start + UNRESPONSIVE_AFTER);
    server.force_quit_tick(Instant::now());
    assert_eq!(
        server.force_quit_rows(),
        ["Editor", "Stuck (Not Responding)"]
    );
    chord(&mut server, &[KEY_DOWN]);
    chord(&mut server, &[KEY_ENTER]);
    assert!(server.force_quit_asks());
    chord(&mut server, &[KEY_ENTER]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while server
        .windows()
        .iter()
        .any(|window| window.app_id() == "org.example.Stuck")
    {
        assert!(Instant::now() < deadline, "the hung app is still running");
        server.dispatch().expect("server dispatch");
        thread::sleep(Duration::from_millis(5));
    }
    server.force_quit_tick(Instant::now());
    roundtrip(&mut server, &mut [&mut editor]);

    assert_eq!(server.force_quit_rows(), ["Editor"]);
    assert!(
        editor
            .keys()
            .iter()
            .all(|(key, _)| ![KEY_ESC, KEY_DOWN, KEY_ENTER].contains(key))
    );
}

#[test]
fn a_window_on_a_workspace_out_of_view_is_suspended() {
    let (mut server, mut editor) = focused_app();
    let mut viewer = TestClient::window(&mut server, "org.example.Viewer");
    server.press_button(BTN_LEFT, true, 0);
    server.press_button(BTN_LEFT, false, 0);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);
    assert!(viewer.keyboard_focus_on(0));
    chord(
        &mut server,
        &[KEY_LEFTMETA, KEY_LEFTCTRL, KEY_LEFTSHIFT, KEY_RIGHT],
    );
    let settle = |server: &mut Compositor, clients: &mut [&mut TestClient]| {
        for frame in 1..120 {
            server.animate(Duration::from_millis(16 * frame));
        }
        roundtrip(server, clients);
    };
    settle(&mut server, &mut [&mut editor, &mut viewer]);
    assert!(!viewer.is_suspended(0));
    assert!(editor.is_suspended(0));
    chord(&mut server, &[KEY_LEFTMETA, KEY_LEFTCTRL, KEY_LEFT]);
    settle(&mut server, &mut [&mut editor, &mut viewer]);
    assert!(!editor.is_suspended(0));
    assert!(viewer.is_suspended(0));
}
