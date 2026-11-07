#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::thread;
use std::time::{Duration, Instant};

use bande_compositor::{
    Compositor, KEYBOARD_MONITOR_NAME, OutputConfig, Renderer, SCREEN_READER_NAME,
    UNRESPONSIVE_AFTER, serve_keyboard_monitor,
};
use bande_config::Config;
use bande_test_bus::SessionBus;
use bande_test_wayland::{
    BLUE, ConstraintAdjustment, PopupRequest, RED, SeparateApp, ShmImage, TestClient,
    act_as_separate_app, chord, headless, roundtrip,
};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use wayland_client::protocol::wl_output::Transform;
use wayland_client::protocol::wl_shm::Format;

const KEY_ESC: u32 = 1;
const KEY_A: u32 = 30;
const KEY_T: u32 = 20;
const KEY_SPACE: u32 = 57;
const KEY_ENTER: u32 = 28;
const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTALT: u32 = 56;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_LEFTMETA: u32 = 125;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_DOWN: u32 = 108;
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
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
fn a_covered_window_that_keeps_drawing_does_not_exhaust_the_gpu_memory() {
    let mut server = headless(560, 520);
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut behind = TestClient::window(&mut server, "org.example.Behind");
    let large = RED.repeat(1000 * 1000);
    let cover = ShmImage {
        width: 1000,
        height: 1000,
        stride: 4000,
        format: Format::Xrgb8888,
        bytes: &large,
    };
    let mut front = TestClient::connect(&mut server);
    front.show_window(&mut server, &cover);
    roundtrip(&mut server, &mut [&mut behind, &mut front]);
    renderer.render(&mut server).expect("first frame");
    let frame = RED.repeat(64 * 64);
    let image = ShmImage {
        width: 64,
        height: 64,
        stride: 256,
        format: Format::Xrgb8888,
        bytes: &frame,
    };
    for _ in 0..2000 {
        behind.commit_buffer(0, &image, (0, 0, 64, 64));
        roundtrip(&mut server, &mut [&mut behind, &mut front]);
        let shown = renderer.render(&mut server).expect("frame");
        assert_eq!(shown.repainted_pixels, 0, "the window is not covered");
    }
    let (_, staging) = renderer.memory_use().staging;
    assert!(staging <= 8 << 20, "{staging} bytes are waiting to upload");
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
fn an_app_that_closed_its_last_window_stays_active_until_the_desktop_is_clicked() {
    let (mut server, mut editor) = focused_app();
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.listen_as_shell(&mut server);
    editor.keep_running(&mut server, "org.example.Editor");
    editor.unmap_window(&mut server, 0);
    roundtrip(&mut server, &mut [&mut shell, &mut editor]);
    let active = |shell: &TestClient| {
        shell
            .shell_events()
            .iter()
            .rfind(|sent| sent.starts_with("ActiveApp"))
            .cloned()
            .unwrap_or_default()
    };
    assert!(active(&shell).contains("org.example.Editor"));
    server.press_button(BTN_LEFT, true, 0);
    server.press_button(BTN_LEFT, false, 0);
    roundtrip(&mut server, &mut [&mut shell, &mut editor]);
    assert!(!active(&shell).contains("org.example.Editor"));
}

#[test]
fn tapping_super_opens_search_and_ctrl_alt_t_opens_terminal() {
    let (mut server, mut editor) = focused_app();
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.listen_as_shell(&mut server);
    let opened = |shell: &TestClient, event: &str| {
        shell
            .shell_events()
            .iter()
            .filter(|sent| sent.starts_with(event))
            .count()
    };
    chord(&mut server, &[KEY_LEFTMETA, KEY_SPACE]);
    roundtrip(&mut server, &mut [&mut shell, &mut editor]);
    assert_eq!(opened(&shell, "ShowLauncher"), 0);
    assert!(editor.keys().contains(&(KEY_SPACE, true)));
    for (key, pressed) in [
        (KEY_SPACE, true),
        (KEY_LEFTMETA, true),
        (KEY_LEFTMETA, false),
        (KEY_SPACE, false),
    ] {
        server.key(key, pressed, 0);
    }
    roundtrip(&mut server, &mut [&mut shell, &mut editor]);
    assert_eq!(opened(&shell, "ShowLauncher"), 0);
    chord(&mut server, &[KEY_LEFTMETA]);
    roundtrip(&mut server, &mut [&mut shell, &mut editor]);
    assert_eq!(opened(&shell, "ShowLauncher"), 1);
    chord(&mut server, &[KEY_LEFTCTRL, KEY_LEFTALT, KEY_T]);
    roundtrip(&mut server, &mut [&mut shell, &mut editor]);
    assert_eq!(opened(&shell, "OpenTerminal"), 1);
    assert!(!editor.keys().contains(&(KEY_T, true)));
}

#[test]
fn a_window_on_a_workspace_out_of_view_is_suspended() {
    let (mut server, mut editor) = focused_app();
    let mut viewer = TestClient::window(&mut server, "org.example.Viewer");
    server.press_button(BTN_LEFT, true, 0);
    server.press_button(BTN_LEFT, false, 0);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);
    assert!(viewer.keyboard_focus_on(0));
    viewer.move_to_next_workspace(&mut server, 0);
    let settle = |server: &mut Compositor, clients: &mut [&mut TestClient]| {
        for frame in 1..120 {
            server.animate(Duration::from_millis(16 * frame));
        }
        roundtrip(server, clients);
    };
    settle(&mut server, &mut [&mut editor, &mut viewer]);
    assert!(!editor.is_suspended(0));
    assert!(viewer.is_suspended(0));
    chord(&mut server, &[KEY_LEFTMETA, KEY_LEFTCTRL, KEY_RIGHT]);
    settle(&mut server, &mut [&mut editor, &mut viewer]);
    assert!(!viewer.is_suspended(0));
    assert!(editor.is_suspended(0));
}

#[test]
fn windows_with_turned_or_flipped_buffers_do_not_stop_the_screen_from_updating() {
    let mut server = headless(560, 520);
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut plain = TestClient::window(&mut server, "org.example.Plain");
    let mut turned = TestClient::window(&mut server, "org.example.Turned");
    let bytes = RED.repeat(64 * 32);
    let image = ShmImage {
        width: 64,
        height: 32,
        stride: 256,
        format: Format::Xrgb8888,
        bytes: &bytes,
    };
    for transform in [
        Transform::_90,
        Transform::_180,
        Transform::_270,
        Transform::Flipped,
        Transform::Flipped90,
        Transform::Flipped180,
        Transform::Flipped270,
    ] {
        turned.set_buffer_transform(0, transform);
        turned.commit_buffer(0, &image, (0, 0, 64, 32));
        roundtrip(&mut server, &mut [&mut plain, &mut turned]);
        renderer
            .render(&mut server)
            .unwrap_or_else(|error| panic!("{transform:?} stopped the screen: {error}"));
    }
}

#[test]
fn a_window_that_resizes_itself_keeps_its_top_left_and_is_remembered_at_its_new_size() {
    let memory = std::env::temp_dir().join(format!("bande-windows-{}", std::process::id()));
    let mut server = headless(560, 520);
    server.remember_windows_in(memory.clone());
    let pixels = RED.repeat(120 * 80);
    let image = |width: i32, height: i32| ShmImage {
        width,
        height,
        stride: width * 4,
        format: Format::Xrgb8888,
        bytes: &pixels[..(width * height * 4) as usize],
    };
    let mut app = TestClient::window(&mut server, "org.example.Sizer");
    app.unmap_window(&mut server, 0);
    app.remap_window(&mut server, 0, &image(64, 64));
    assert_eq!(app.configured_size(0), Some((64, 64)));
    let top_left = server.windows()[0].geometry().unwrap().loc;

    app.commit_buffer(0, &image(120, 80), (0, 0, 120, 80));
    let mut other = TestClient::window(&mut server, "org.example.Other");
    roundtrip(&mut server, &mut [&mut app, &mut other]);
    let resized = server.windows()[0].geometry().unwrap();
    assert_eq!(
        (resized.loc, (resized.size.w, resized.size.h)),
        (top_left, (120, 80))
    );
    assert_eq!(app.configured_size(0), Some((0, 0)));

    app.unmap_window(&mut server, 0);
    app.remap_window(&mut server, 0, &image(120, 80));
    std::fs::remove_file(memory).ok();
    assert_eq!(app.configured_size(0), Some((120, 80)));
    assert_eq!(server.windows()[0].geometry().unwrap().loc, top_left);
}

fn take_configured_sizes(server: &mut Compositor, clients: &mut [&mut TestClient]) {
    roundtrip(server, clients);
    for client in clients.iter_mut() {
        let (width, height) = client.configured_size(0).unwrap();
        let pixels = RED.repeat((width * height) as usize);
        let image = ShmImage {
            width,
            height,
            stride: width * 4,
            format: Format::Xrgb8888,
            bytes: &pixels,
        };
        client.commit_buffer(0, &image, (0, 0, width, height));
    }
    roundtrip(server, clients);
}

fn drag(server: &mut Compositor, button: u32, from: (f64, f64), to: (f64, f64)) {
    server.move_pointer_to(from.into(), 1);
    server.press_button(button, true, 2);
    server.move_pointer_to(to.into(), 3);
    server.press_button(button, false, 4);
}

fn frame_of(server: &Compositor, app_id: &str) -> (i32, i32, i32, i32) {
    let window = server
        .windows()
        .iter()
        .find(|window| window.app_id() == app_id)
        .unwrap();
    let rect = window.geometry().unwrap();
    (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h)
}

fn tiled(server: &Compositor, app_id: &str) -> bool {
    server
        .windows()
        .iter()
        .any(|window| window.app_id() == app_id && window.tile().is_some())
}

#[test]
fn a_snapped_window_only_takes_the_shape_and_then_resizes_and_moves_like_any_window() {
    const LEFT: &str = "org.example.Left";
    const RIGHT: &str = "org.example.Right";
    let mut server = headless(1000, 800);
    let mut left = TestClient::window(&mut server, LEFT);
    chord(&mut server, &[KEY_LEFTMETA, KEY_RIGHT]);
    take_configured_sizes(&mut server, &mut [&mut left]);
    assert_eq!(frame_of(&server, LEFT), (500, 0, 500, 800));
    assert!(
        !server.windows()[0].has_state(xdg_toplevel::State::TiledRight),
        "clients drop their resize edges in a tiled state"
    );
    chord(&mut server, &[KEY_LEFTMETA, KEY_LEFT]);
    take_configured_sizes(&mut server, &mut [&mut left]);
    assert_eq!(frame_of(&server, LEFT), (0, 0, 500, 800));

    let mut right = TestClient::window(&mut server, RIGHT);
    let unsnapped = frame_of(&server, RIGHT);
    chord(&mut server, &[KEY_LEFTMETA, KEY_RIGHT]);
    take_configured_sizes(&mut server, &mut [&mut left, &mut right]);
    assert_eq!(frame_of(&server, RIGHT), (500, 0, 500, 800));

    drag(&mut server, BTN_LEFT, (497.0, 400.0), (397.0, 400.0));
    take_configured_sizes(&mut server, &mut [&mut left, &mut right]);
    assert_eq!(frame_of(&server, LEFT), (0, 0, 400, 800));
    assert_eq!(frame_of(&server, RIGHT), (400, 0, 600, 800));
    assert!(tiled(&server, LEFT) && tiled(&server, RIGHT));

    server.key(KEY_LEFTMETA, true, 0);
    drag(&mut server, BTN_RIGHT, (200.0, 700.0), (200.0, 600.0));
    take_configured_sizes(&mut server, &mut [&mut left, &mut right]);
    assert_eq!(frame_of(&server, LEFT), (0, 0, 400, 700));
    assert!(!tiled(&server, LEFT) && tiled(&server, RIGHT));

    drag(&mut server, BTN_LEFT, (200.0, 300.0), (300.0, 350.0));
    take_configured_sizes(&mut server, &mut [&mut left, &mut right]);
    assert_eq!(frame_of(&server, LEFT), (100, 50, 400, 700));

    drag(&mut server, BTN_LEFT, (800.0, 300.0), (800.0, 350.0));
    server.key(KEY_LEFTMETA, false, 5);
    take_configured_sizes(&mut server, &mut [&mut left, &mut right]);
    let (_, _, width, height) = frame_of(&server, RIGHT);
    assert_eq!((width, height), (unsnapped.2, unsnapped.3));
    assert!(!tiled(&server, RIGHT));
}

fn frame_of_only_window(
    server: &Compositor,
) -> smithay::utils::Rectangle<i32, smithay::utils::Logical> {
    server.windows()[0].geometry().unwrap()
}

fn monitor(name: &str, serial: &str, width: i32, height: i32) -> OutputConfig {
    OutputConfig {
        serial: serial.to_owned(),
        ..OutputConfig::virtual_display(name, (width, height).into())
    }
}

#[test]
fn the_only_display_can_blink_or_be_swapped_and_the_windows_end_up_where_they_were() {
    let mut server = headless(1000, 800);
    let mut app = TestClient::window(&mut server, "org.example.Editor");
    let frame = frame_of_only_window(&server);
    let first = monitor("HEADLESS-1", "HEADLESS-1", 1000, 800);

    server.remove_output("HEADLESS-1");
    assert_eq!(server.outputs(), ["HEADLESS-1"]);
    assert!(!server.output_powered("HEADLESS-1"));
    assert_eq!(frame_of_only_window(&server), frame);
    assert!(server.connected_displays(&mut |_| false).is_empty());
    server.add_output(first.clone());
    roundtrip(&mut server, &mut [&mut app]);
    assert!(server.output_powered("HEADLESS-1"));
    assert_eq!(frame_of_only_window(&server), frame);

    server.remove_output("HEADLESS-1");
    server.add_output(monitor("HEADLESS-1", "another", 1600, 900));
    roundtrip(&mut server, &mut [&mut app]);
    assert_eq!(server.outputs(), ["HEADLESS-1"]);
    assert!(server.output_powered("HEADLESS-1"));
    assert_eq!(
        server.output_size(),
        smithay::utils::Size::from((1600, 900))
    );
    assert_ne!(frame_of_only_window(&server), frame);

    server.remove_output("HEADLESS-1");
    server.add_output(first);
    roundtrip(&mut server, &mut [&mut app]);
    assert_eq!(
        server.output_size(),
        smithay::utils::Size::from((1000, 800))
    );
    assert_eq!(frame_of_only_window(&server), frame);
}

#[test]
fn a_second_display_keeps_its_scale_place_and_windows_across_a_replug() {
    let mut server = headless(1000, 800);
    let mut app = TestClient::window(&mut server, "org.example.Editor");
    let second = monitor("HDMI-A-2", "second", 1600, 1200);
    let config = Config::parse(&format!(
        "display \"{}\" {{\n    scale 2\n}}\n",
        second.identity()
    ))
    .unwrap();
    server.set_display_config(config);
    server.add_output(second.clone());
    let settled = server.output_settings("HDMI-A-2").unwrap();
    assert_eq!(settled.scale, 2.0);

    let frame = frame_of_only_window(&server);
    let inside = (
        f64::from(frame.loc.x) + f64::from(frame.size.w) / 2.0,
        f64::from(frame.loc.y) + f64::from(frame.size.h) / 2.0,
    );
    server.move_pointer_to(inside.into(), 0);
    server.press_button(BTN_LEFT, true, 0);
    server.press_button(BTN_LEFT, false, 0);
    roundtrip(&mut server, &mut [&mut app]);
    chord(&mut server, &[KEY_LEFTSHIFT, KEY_LEFTMETA, KEY_RIGHT]);
    roundtrip(&mut server, &mut [&mut app]);
    let there = frame_of_only_window(&server);
    assert!(there.loc.x >= settled.position.x);

    server.remove_output("HDMI-A-2");
    roundtrip(&mut server, &mut [&mut app]);
    assert_eq!(server.outputs(), ["HEADLESS-1"]);
    assert!(frame_of_only_window(&server).loc.x < 1000);

    server.add_output(second);
    roundtrip(&mut server, &mut [&mut app]);
    assert_eq!(server.output_settings("HDMI-A-2").unwrap(), settled);
    assert_eq!(frame_of_only_window(&server), there);
}
