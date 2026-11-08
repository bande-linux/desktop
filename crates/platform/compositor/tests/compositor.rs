#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::thread;
use std::time::{Duration, Instant};

use bande_compositor::{
    Compositor, KEYBOARD_MONITOR_NAME, SCREEN_READER_NAME, UNRESPONSIVE_AFTER,
    serve_keyboard_monitor,
};
use bande_test_bus::SessionBus;
use bande_test_wayland::keys::{
    BTN_LEFT, KEY_A, KEY_DOWN, KEY_ENTER, KEY_ESC, KEY_LEFTALT, KEY_LEFTCTRL, KEY_Q, KEY_W,
};
use bande_test_wayland::{
    BLUE, ConstraintAdjustment, PopupRequest, RED, ResizeEdge, SeparateApp, ShmImage, TestClient,
    act_as_separate_app, chord, grant_consent, headless, roundtrip,
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
    let mut lock = TestClient::connect_as_lock_screen(server);
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
fn a_window_edge_keeps_the_resize_pointer_while_it_is_dragged() {
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
    let rect = server.windows()[0].geometry().expect("mapped");
    let middle = f64::from(rect.loc.y + rect.size.h / 2);
    let right = f64::from(rect.loc.x + rect.size.w);
    server.move_pointer_to((right - 100.0, middle).into(), 0);
    roundtrip(&mut server, &mut [&mut browser]);
    server.move_pointer_to((right + 3.0, middle).into(), 0);
    assert_eq!(server.shown_cursor(), Some("ew-resize"));
    server.press_button(BTN_LEFT, true, 0);
    server.move_pointer_to((right + 40.0, middle).into(), 0);
    roundtrip(&mut server, &mut [&mut browser]);
    assert_eq!(server.shown_cursor(), Some("ew-resize"));
    server.press_button(BTN_LEFT, false, 0);

    let mut server = headless(560, 520);
    let mut editor = TestClient::window(&mut server, "org.example.Editor");
    let rect = server.windows()[0].geometry().expect("mapped");
    let centre = rect.loc.to_f64() + rect.size.downscale(2).to_point().to_f64();
    server.move_pointer_to(centre, 0);
    server.press_button(BTN_LEFT, true, 0);
    roundtrip(&mut server, &mut [&mut editor]);
    editor.start_resize(&mut server, 0, ResizeEdge::Right);
    server.move_pointer_to((centre.x + 20.0, centre.y).into(), 0);
    roundtrip(&mut server, &mut [&mut editor]);
    assert_eq!(server.shown_cursor(), Some("ew-resize"));
    server.press_button(BTN_LEFT, false, 0);
    roundtrip(&mut server, &mut [&mut editor]);
    assert_eq!(server.shown_cursor(), Some("default"));
}

#[test]
fn while_locked_no_popup_grab_or_input_method_takes_keys_from_the_lock_surface() {
    let (mut server, mut editor) = focused_app();
    let _consent = grant_consent(&mut server, "read-keys sandboxed org.example.Ime\n");
    let mut input_method = TestClient::connect_sandboxed(&mut server, "org.example.Ime");
    input_method.use_seat(&mut server);
    input_method.become_input_method(&mut server);
    assert!(!input_method.input_method_unavailable());
    editor.enable_text_input(&mut server);
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
fn without_consent_no_program_reads_keys_or_types() {
    let (mut server, mut editor) = focused_app();
    let mut spy = TestClient::connect_sandboxed(&mut server, "org.example.Spy");
    spy.use_seat(&mut server);
    spy.become_input_method(&mut server);
    assert!(spy.input_method_unavailable());
    spy.grab_keyboard(&mut server);

    chord(&mut server, &[KEY_A]);
    roundtrip(&mut server, &mut [&mut spy, &mut editor]);
    assert!(spy.grabbed_keys().is_empty());
    assert!(editor.keys().contains(&(KEY_A, true)));

    let mut typist = TestClient::connect(&mut server);
    typist.use_seat(&mut server);
    assert!(typist.create_virtual_keyboard(&mut server).is_err());
}

#[test]
fn a_consented_input_method_sees_keys_only_in_a_text_field_that_is_no_password() {
    let (mut server, mut editor) = focused_app();
    let _consent = grant_consent(&mut server, "read-keys sandboxed org.example.Ime\n");
    let mut input_method = TestClient::connect_sandboxed(&mut server, "org.example.Ime");
    input_method.use_seat(&mut server);
    input_method.become_input_method(&mut server);
    input_method.grab_keyboard(&mut server);

    chord(&mut server, &[KEY_A]);
    roundtrip(&mut server, &mut [&mut input_method, &mut editor]);
    assert!(input_method.grabbed_keys().is_empty());

    editor.enable_text_input(&mut server);
    input_method.grab_keyboard(&mut server);
    chord(&mut server, &[KEY_Q]);
    roundtrip(&mut server, &mut [&mut input_method, &mut editor]);
    assert!(input_method.grabbed_keys().contains(&(KEY_Q, true)));
    assert!(!editor.keys().contains(&(KEY_Q, true)));

    editor.mark_text_input_as_password(&mut server);
    chord(&mut server, &[KEY_W]);
    roundtrip(&mut server, &mut [&mut input_method, &mut editor]);
    assert!(!input_method.grabbed_keys().contains(&(KEY_W, true)));
    assert!(editor.keys().contains(&(KEY_W, true)));
}

#[test]
fn while_locked_a_consented_virtual_keyboard_reaches_nothing() {
    let (mut server, mut editor) = focused_app();
    let _consent = grant_consent(&mut server, "type-keys sandboxed org.example.Typist\n");
    let mut typist = TestClient::connect_sandboxed(&mut server, "org.example.Typist");
    typist.use_seat(&mut server);
    assert!(typist.create_virtual_keyboard(&mut server).is_ok());
    typist.type_on_virtual_keyboard(&mut server, KEY_A);
    roundtrip(&mut server, &mut [&mut typist, &mut editor]);
    assert!(editor.keys().contains(&(KEY_A, true)));

    let mut lock = locked(&mut server);
    roundtrip(&mut server, &mut [&mut lock, &mut typist, &mut editor]);
    assert!(lock.lock_surface_has_keyboard());
    typist.type_on_virtual_keyboard(&mut server, KEY_Q);
    roundtrip(&mut server, &mut [&mut lock, &mut typist, &mut editor]);
    assert!(!lock.keys().contains(&(KEY_Q, true)));
    assert!(!editor.keys().contains(&(KEY_Q, true)));
}

#[test]
fn turning_consent_off_cuts_the_program_off_and_keys_go_to_the_app_at_once() {
    let (mut server, mut editor) = focused_app();
    let consent = grant_consent(&mut server, "read-keys sandboxed org.example.Ime\n");
    let mut input_method = TestClient::connect_sandboxed(&mut server, "org.example.Ime");
    input_method.use_seat(&mut server);
    input_method.become_input_method(&mut server);
    assert!(!input_method.input_method_unavailable());
    editor.enable_text_input(&mut server);
    input_method.grab_keyboard(&mut server);
    chord(&mut server, &[KEY_A]);
    roundtrip(&mut server, &mut [&mut input_method, &mut editor]);
    assert!(input_method.grabbed_keys().contains(&(KEY_A, true)));

    let replacement = consent.path().join("replacement");
    std::fs::write(&replacement, "").unwrap();
    std::fs::rename(&replacement, consent.path().join("consent")).unwrap();
    server.dispatch().unwrap();
    chord(&mut server, &[KEY_Q]);
    roundtrip(&mut server, &mut [&mut editor]);
    input_method.read_what_arrived();

    assert!(editor.keys().contains(&(KEY_Q, true)));
    assert!(!input_method.grabbed_keys().contains(&(KEY_Q, true)));
}

#[test]
fn only_the_lock_screen_may_lock_and_a_sandbox_gets_no_privileged_protocol() {
    let mut server = headless(560, 520);
    let shell = TestClient::connect_as_shell(&mut server);
    assert!(!shell.sees_global("ext_session_lock_manager_v1"));
    let lock = TestClient::connect_as_lock_screen(&mut server);
    assert!(lock.sees_global("ext_session_lock_manager_v1"));
    let sandboxed = TestClient::connect_sandboxed(&mut server, "org.example.App");
    for privileged in [
        "ext_session_lock_manager_v1",
        "bande_app_control_v1",
        "zwlr_layer_shell_v1",
        "zwlr_output_manager_v1",
        "zwlr_output_power_manager_v1",
        "ext_idle_notifier_v1",
        "ext_data_control_manager_v1",
        "ext_image_copy_capture_manager_v1",
        "wp_security_context_manager_v1",
    ] {
        assert!(!sandboxed.sees_global(privileged), "{privileged}");
    }
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

#[test]
fn a_click_beside_the_search_reaches_the_window_and_the_search_stays_where_it_was_dragged() {
    let mut server = headless(800, 600);
    let mut editor = TestClient::window(&mut server, "org.example.Editor");
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.use_seat(&mut server);
    let bytes = BLUE.repeat(200 * 100);
    let card = ShmImage {
        width: 200,
        height: 100,
        stride: 800,
        format: Format::Xrgb8888,
        bytes: &bytes,
    };
    shell.show_panel(&mut server, &card, 100);
    let rect = server.windows()[0].geometry().expect("mapped");
    let window = rect.loc.to_f64() + rect.size.downscale(2).to_point().to_f64();
    let clicks = editor.buttons().len();
    let click = |server: &mut Compositor,
                 shell: &mut TestClient,
                 editor: &mut TestClient,
                 at: (f64, f64)| {
        server.move_pointer_to(at.into(), 0);
        server.press_button(BTN_LEFT, true, 0);
        server.press_button(BTN_LEFT, false, 0);
        roundtrip(server, &mut [shell, editor]);
    };
    click(&mut server, &mut shell, &mut editor, (window.x, window.y));
    assert_eq!(shell.pressed_outside(), 1);
    assert_eq!(editor.buttons().len(), clicks + 2);

    let drag =
        |server: &mut Compositor, shell: &mut TestClient, from: (f64, f64), to: (f64, f64)| {
            server.move_pointer_to(from.into(), 0);
            server.press_button(BTN_LEFT, true, 0);
            shell.exchange(server);
            shell.move_panel(server);
            server.move_pointer_to(to.into(), 0);
            assert_eq!(server.shown_cursor(), Some("default"));
            server.press_button(BTN_LEFT, false, 0);
            shell.exchange(server);
        };
    drag(&mut server, &mut shell, (400.0, 150.0), (150.0, 450.0));
    click(&mut server, &mut shell, &mut editor, (150.0, 450.0));
    assert_eq!(shell.pressed_outside(), 1);
    shell.close_panel(&mut server);
    shell.show_panel(&mut server, &card, 100);
    click(&mut server, &mut shell, &mut editor, (150.0, 450.0));
    assert_eq!(shell.pressed_outside(), 1);
    click(&mut server, &mut shell, &mut editor, (400.0, 150.0));
    assert_eq!(shell.pressed_outside(), 2);

    drag(&mut server, &mut shell, (150.0, 450.0), (400.0, 155.0));
    click(&mut server, &mut shell, &mut editor, (400.0, 202.0));
    assert_eq!(shell.pressed_outside(), 3);
}
