#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::thread;
use std::time::{Duration, Instant};

use bande_compositor::{
    Compositor, KEYBOARD_MONITOR_NAME, SCREEN_READER_NAME, UNRESPONSIVE_AFTER,
    serve_keyboard_monitor,
};
use bande_test_bus::SessionBus;
use bande_test_wayland::keys::{
    BTN_LEFT, KEY_A, KEY_DOWN, KEY_ENTER, KEY_ESC, KEY_LEFTALT, KEY_LEFTCTRL,
};
use bande_test_wayland::{
    BLUE, ConstraintAdjustment, Nv12Frame, PopupRequest, RED, SeparateApp, ShmImage, TestClient,
    act_as_separate_app, chord, headless, roundtrip,
};
use wayland_client::protocol::wl_shm::Format;

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
fn the_browser_row_gives_the_well_to_the_compositor_and_the_rest_to_the_browser() {
    let mut server = headless(560, 520);
    let bytes = RED.repeat(320 * 200);
    let mut browser = TestClient::connect(&mut server);
    browser.use_seat(&mut server);
    browser.show_window(
        &mut server,
        &ShmImage {
            width: 320,
            height: 200,
            stride: 1280,
            format: Format::Xrgb8888,
            bytes: &bytes,
        },
    );
    browser.set_app_id(&mut server, 0, "org.bande.Browser");
    roundtrip(&mut server, &mut [&mut browser]);
    let at = server.windows()[0].geometry().expect("mapped").loc;
    let click = |server: &mut Compositor, x: i32| {
        server.move_pointer_to((f64::from(at.x + x), f64::from(at.y + 24)).into(), 0);
        server.press_button(BTN_LEFT, true, 0);
        server.press_button(BTN_LEFT, false, 0);
    };
    click(&mut server, 90);
    roundtrip(&mut server, &mut [&mut browser]);
    assert_eq!(browser.buttons(), [(BTN_LEFT, true), (BTN_LEFT, false)]);
    click(&mut server, 45);
    roundtrip(&mut server, &mut [&mut browser]);
    assert_eq!(browser.buttons().len(), 2);
    assert!(server.windows()[0].is_minimized());
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
fn a_hung_app_is_force_quit_from_the_keyboard() {
    if act_as_separate_app() {
        return;
    }
    let mut server = headless(560, 520);
    let mut bar = TestClient::connect_as_shell(&mut server);
    bar.join_shell(&mut server);
    let mut stuck = SeparateApp::launch(&mut server, "a_hung_app_is_force_quit_from_the_keyboard");
    stuck.show_window_of(&mut server, 64, "org.example.Stuck");
    let mut editor = TestClient::window(&mut server, "org.example.Editor");
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
fn an_nv12_video_buffer_shows_its_colour() {
    let mut server = headless(400, 300);
    let mut renderer = bande_compositor::Renderer::new().expect("Vulkan renderer");
    let mut client = TestClient::connect(&mut server);
    let red = Nv12Frame {
        width: 128,
        height: 96,
        luma: 63,
        blue_difference: 102,
        red_difference: 240,
    };
    if !client.show_nv12(&mut server, &red) {
        return;
    }
    let mut seen = Vec::new();
    for _ in 0..50 {
        renderer.render(&mut server).expect("render");
        client.exchange(&mut server);
        let snapshot = renderer.snapshot(&server).expect("snapshot");
        seen = snapshot.rgba;
        if seen
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] > 240 && pixel[1] < 20 && pixel[2] < 20)
        {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let brightest = seen
        .as_chunks::<4>()
        .0
        .iter()
        .max_by_key(|pixel| pixel[0])
        .map(|pixel| pixel.to_vec());
    panic!("no red in the frame, brightest {brightest:?}");

fn another_process_dialog_opens_over_the_window_it_is_attached_to() {
    let mut server = headless(1200, 900);
    let bytes = RED.repeat(600 * 500);
    let mut app = TestClient::connect(&mut server);
    app.show_window(
        &mut server,
        &ShmImage {
            width: 600,
            height: 500,
            stride: 2400,
            format: Format::Xrgb8888,
            bytes: &bytes,
        },
    );
    app.set_app_id(&mut server, 0, "org.example.Editor");
    let handle = app.export_window(&mut server, 0);

    let bytes = BLUE.repeat(200 * 100);
    let mut dialogs = TestClient::connect(&mut server);
    dialogs.show_window_attached_to(
        &mut server,
        &ShmImage {
            width: 200,
            height: 100,
            stride: 800,
            format: Format::Xrgb8888,
            bytes: &bytes,
        },
        &handle,
    );
    dialogs.set_app_id(&mut server, 0, "org.bande.FileChooser");
    roundtrip(&mut server, &mut [&mut app, &mut dialogs]);

    app.minimize_window(&mut server, 0);
    roundtrip(&mut server, &mut [&mut app, &mut dialogs]);
    assert!(
        server
            .windows()
            .iter()
            .find(|window| window.app_id() == "org.bande.FileChooser")
            .unwrap()
            .is_minimized()
    );
}
