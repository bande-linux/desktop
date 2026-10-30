#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use smithay::utils::{Logical, Point, Size};
use std::time::Duration;

use unbound_compositor::{Headless, OutputConfig, OutputMode, Rect, Renderer};
use unbound_config::Config;
use unbound_test_wayland::{
    Anchor, BLUE, DecorationMode, GREEN, KeyboardInteractivity, Layer, LayerRequest, RED, ShmImage,
    TestClient, quadrants, roundtrip,
};

use wayland_client::protocol::wl_shm::Format;

const MAIN: &str = "HEADLESS-1";
const SIDE: &str = "HEADLESS-2";
const MAIN_SIZE: (i32, i32) = (256, 192);
const SIDE_SIZE: (i32, i32) = (320, 240);
const WINDOW: (i32, i32) = (64, 48);
const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_LEFTALT: u32 = 56;
const KEY_LEFTMETA: u32 = 125;
const KEY_UP: u32 = 103;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const BTN_LEFT: u32 = 0x110;

fn side_display() -> OutputConfig {
    OutputConfig::virtual_display(SIDE, Size::from(SIDE_SIZE))
}

fn two_displays() -> Headless {
    let mut server = Headless::new(Size::from(MAIN_SIZE)).expect("headless compositor");
    server.add_output(side_display());
    server
}

fn at(x: f64, y: f64) -> Point<f64, Logical> {
    (x, y).into()
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect {
        location: (x, y).into(),
        size: (w, h).into(),
    }
}

fn app(server: &mut Headless) -> TestClient {
    let mut client = TestClient::connect(server);
    client.use_seat(server);
    let pixels = RED.repeat((WINDOW.0 * WINDOW.1) as usize);
    client.show_window(
        server,
        &ShmImage {
            width: WINDOW.0,
            height: WINDOW.1,
            stride: WINDOW.0 * 4,
            format: Format::Xrgb8888,
            bytes: &pixels,
        },
    );
    client
}

fn chord(server: &mut Headless, modifiers: &[u32], key: u32) {
    for modifier in modifiers {
        server.key(*modifier, true, 0);
    }
    server.key(key, true, 0);
    server.key(key, false, 0);
    for modifier in modifiers.iter().rev() {
        server.key(*modifier, false, 0);
    }
}

fn geometry(server: &Headless) -> Rect {
    server.windows()[0].geometry().expect("mapped")
}

fn red_pixels(renderer: &mut Renderer, server: &mut Headless, output: &str) -> usize {
    renderer.render_output(server, output).expect("frame");
    let snapshot = renderer.snapshot_output(output).expect("read back");
    snapshot
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[..3] == [0xff, 0, 0])
        .count()
}

#[test]
fn a_new_display_is_placed_to_the_right_of_the_main_one() {
    let server = two_displays();

    assert_eq!(server.outputs(), [MAIN, SIDE]);
    assert_eq!(server.output_rect(SIDE), Some(rect(256, 0, 320, 240)));
}

#[test]
fn the_pointer_crosses_between_displays_but_never_leaves_them() {
    let mut server = two_displays();

    server.move_pointer_to(at(300.0, 100.0), 0);
    assert_eq!(server.pointer_location(), at(300.0, 100.0));
    server.move_pointer_to(at(300.0, 230.0), 0);
    assert_eq!(server.pointer_location(), at(300.0, 230.0));
    server.move_pointer_to(at(100.0, 230.0), 0);
    assert_eq!(server.pointer_location(), at(100.0, 191.0));
}

#[test]
fn a_window_opens_centred_on_the_display_with_the_pointer() {
    let mut server = two_displays();
    server.move_pointer_to(at(400.0, 100.0), 0);

    let _app = app(&mut server);

    assert_eq!(server.output_of(&server.windows()[0]), SIDE);
    assert_eq!(geometry(&server), rect(256 + 128, 96, 64, 48));
}

#[test]
fn each_display_draws_only_its_own_windows() {
    let mut server = two_displays();
    server.move_pointer_to(at(400.0, 100.0), 0);
    let _app = app(&mut server);
    let mut renderer = Renderer::new().expect("renderer");

    assert!(red_pixels(&mut renderer, &mut server, SIDE) > (WINDOW.0 * WINDOW.1) as usize * 9 / 10);

    assert_eq!(red_pixels(&mut renderer, &mut server, MAIN), 0);
}

#[test]
fn a_shortcut_moves_the_focused_window_to_the_neighbouring_display_keeping_its_place() {
    let mut server = two_displays();
    let mut app = app(&mut server);
    assert_eq!(geometry(&server), rect(96, 72, 64, 48));

    chord(
        &mut server,
        &[KEY_LEFTALT, KEY_LEFTMETA, KEY_LEFTSHIFT],
        KEY_RIGHT,
    );
    roundtrip(&mut server, &mut [&mut app]);

    assert_eq!(server.output_of(&server.windows()[0]), SIDE);
    assert_eq!(geometry(&server), rect(256 + 128, 96, 64, 48));
    assert!(app.keyboard_focus_on(0));

    chord(
        &mut server,
        &[KEY_LEFTALT, KEY_LEFTMETA, KEY_LEFTSHIFT],
        KEY_LEFT,
    );

    assert_eq!(server.output_of(&server.windows()[0]), MAIN);
    assert_eq!(geometry(&server), rect(96, 72, 64, 48));
}

#[test]
fn dragging_a_window_onto_another_display_moves_it_there() {
    let mut server = two_displays();
    let mut app = app(&mut server);
    server.move_pointer_to(at(100.0, 80.0), 0);
    server.press_button(BTN_LEFT, true, 0);
    roundtrip(&mut server, &mut [&mut app]);
    app.request_move(&mut server, 0);

    server.move_pointer_to(at(300.0, 80.0), 10);
    server.press_button(BTN_LEFT, false, 20);
    roundtrip(&mut server, &mut [&mut app]);

    assert_eq!(server.output_of(&server.windows()[0]), SIDE);
    assert_eq!(geometry(&server).location, (296, 72).into());
}

#[test]
fn windows_of_an_unplugged_display_come_over_and_go_back_when_it_returns() {
    let mut server = two_displays();
    server.move_pointer_to(at(300.0, 20.0), 0);
    let mut app = app(&mut server);
    assert_eq!(geometry(&server), rect(384, 96, 64, 48));

    server.remove_output(SIDE);
    roundtrip(&mut server, &mut [&mut app]);

    assert_eq!(server.outputs(), [MAIN]);
    assert_eq!(server.output_of(&server.windows()[0]), MAIN);
    let moved = geometry(&server);
    assert_eq!(moved.size, WINDOW.into());
    assert!(moved.location.x >= 0 && moved.location.x + WINDOW.0 <= MAIN_SIZE.0);
    assert!(moved.location.y >= 0 && moved.location.y + WINDOW.1 <= MAIN_SIZE.1);

    server.add_output(side_display());
    roundtrip(&mut server, &mut [&mut app]);

    assert_eq!(server.output_of(&server.windows()[0]), SIDE);
    assert_eq!(geometry(&server), rect(384, 96, 64, 48));
}

#[test]
fn each_display_has_its_own_row_of_workspaces() {
    let mut server = two_displays();
    server.move_pointer_to(at(400.0, 100.0), 0);
    let mut app = app(&mut server);

    chord(
        &mut server,
        &[KEY_LEFTCTRL, KEY_LEFTMETA, KEY_LEFTSHIFT],
        KEY_RIGHT,
    );
    roundtrip(&mut server, &mut [&mut app]);

    assert_eq!(server.workspace_count_on(SIDE), Some(2));
    assert_eq!(server.active_workspace_on(SIDE), Some(1));
    assert_eq!(server.workspace_count_on(MAIN), Some(1));
    assert_eq!(server.active_workspace_on(MAIN), Some(0));
}

#[test]
fn a_window_is_told_the_scale_of_the_display_it_is_on() {
    let mut server = two_displays();
    let mut settings = server.output_settings(SIDE).expect("side display");
    settings.scale = 2.0;
    server.configure_output(SIDE, settings);
    assert_eq!(server.output_rect(SIDE), Some(rect(256, 0, 160, 120)));
    let mut app = app(&mut server);
    assert_eq!(app.preferred_scale(&mut server, 0), Some(120));

    chord(
        &mut server,
        &[KEY_LEFTALT, KEY_LEFTMETA, KEY_LEFTSHIFT],
        KEY_RIGHT,
    );

    assert_eq!(app.preferred_scale(&mut server, 0), Some(240));
}

fn config(server: &Headless, arrangement: &str) -> Config {
    let main = server.output_identity(MAIN).expect("main identity");
    let side = server.output_identity(SIDE).expect("side identity");
    let source = arrangement.replace("MAIN", &main).replace("SIDE", &side);
    Config::parse(&source).expect("valid config")
}

#[test]
fn a_remembered_arrangement_places_the_displays_and_names_the_main_one() {
    let mut server = two_displays();

    let arranged = config(
        &server,
        "arrangement {\n    display \"SIDE\" x=0 y=0 main=#true\n    display \"MAIN\" x=320 y=48\n}\n",
    );
    server.set_display_config(arranged);

    assert_eq!(server.main_output(), SIDE);
    assert_eq!(server.output_rect(SIDE), Some(rect(0, 0, 320, 240)));
    assert_eq!(server.output_rect(MAIN), Some(rect(320, 48, 256, 192)));
}

#[test]
fn a_display_unknown_to_the_remembered_arrangement_goes_to_the_right() {
    let mut server = Headless::new(Size::from(MAIN_SIZE)).expect("headless compositor");
    let main = server.output_identity(MAIN).expect("main identity");
    let source = format!("arrangement {{\n    display \"{main}\" x=0 y=0 main=#true\n}}\n");
    server.set_display_config(Config::parse(&source).expect("valid config"));

    server.add_output(side_display());

    assert_eq!(server.output_rect(SIDE), Some(rect(256, 0, 320, 240)));
}

#[test]
fn display_settings_pick_the_resolution_refresh_and_scale() {
    let mut server = Headless::new(Size::from(MAIN_SIZE)).expect("headless compositor");
    let mode = |width: i32, height: i32, hertz: u64| OutputMode {
        size: (width, height).into(),
        refresh: Duration::from_nanos(1_000_000_000 / hertz),
    };
    server.add_output(OutputConfig {
        modes: vec![mode(640, 480, 60), mode(320, 240, 120), mode(320, 240, 60)],
        ..side_display()
    });
    let side = server.output_identity(SIDE).expect("side identity");
    let source = format!(
        "display \"{side}\" {{\n    resolution 320 240\n    refresh-rate 60\n    scale 2\n}}\n"
    );

    server.set_display_config(Config::parse(&source).expect("valid config"));

    let settings = server.output_settings(SIDE).expect("side display");
    assert_eq!(settings.mode, mode(320, 240, 60));
    assert_eq!(server.output_rect(SIDE), Some(rect(256, 0, 160, 120)));
}

#[test]
fn windows_move_with_their_display_when_it_is_rearranged() {
    let mut server = two_displays();
    server.move_pointer_to(at(400.0, 100.0), 0);
    let _app = app(&mut server);

    let arranged = config(
        &server,
        "arrangement {\n    display \"MAIN\" x=0 y=0 main=#true\n    display \"SIDE\" x=-320 y=-40\n}\n",
    );
    server.set_display_config(arranged);

    assert_eq!(server.output_rect(SIDE), Some(rect(-320, -40, 320, 240)));
    assert_eq!(server.output_of(&server.windows()[0]), SIDE);
    assert_eq!(geometry(&server), rect(-320 + 128, -40 + 96, 64, 48));
}

#[test]
fn a_mirroring_display_shows_its_source_scaled_to_fit_and_keeps_no_windows() {
    let mut server = two_displays();
    server.move_pointer_to(at(400.0, 100.0), 0);
    let _side_app = app(&mut server);
    server.move_pointer_to(at(100.0, 100.0), 0);
    let _main_app = app(&mut server);
    let mut renderer = Renderer::new().expect("renderer");

    let arranged = config(
        &server,
        "arrangement {\n    display \"MAIN\" x=0 y=0 main=#true\n    display \"SIDE\" mirror=\"MAIN\"\n}\n",
    );
    server.set_display_config(arranged);

    assert!(
        server
            .windows()
            .iter()
            .all(|window| server.output_of(window) == MAIN)
    );
    server.move_pointer_to(at(400.0, 100.0), 0);
    assert_eq!(server.pointer_location(), at(255.0, 100.0));
    let window = (WINDOW.0 * WINDOW.1) as f64;
    let main = red_pixels(&mut renderer, &mut server, MAIN) as f64;
    let side = red_pixels(&mut renderer, &mut server, SIDE) as f64;
    assert!(main > window * 0.9);
    assert!(side > main * 1.25 * 1.25 * 0.9, "{main} {side}");
}

#[test]
fn while_locked_a_display_without_a_lock_surface_shows_only_its_wallpaper() {
    let mut server = two_displays();
    server.move_pointer_to(at(400.0, 100.0), 0);
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.show_named_layer(
        &mut server,
        &LayerRequest {
            layer: Layer::Background,
            anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
            size: (0, 0),
            exclusive_zone: -1,
            keyboard: KeyboardInteractivity::None,
            color: GREEN,
        },
        "wallpaper",
    );
    let _app = app(&mut server);
    let mut lock = TestClient::connect_as_shell(&mut server);
    lock.use_seat(&mut server);
    lock.lock_session(&mut server);
    lock.show_lock_surface(&mut server, BLUE);
    let mut renderer = Renderer::new().expect("renderer");

    assert_eq!(red_pixels(&mut renderer, &mut server, SIDE), 0);
    let snapshot = renderer.snapshot_output(SIDE).expect("read back");
    assert_eq!(snapshot.rgba[..3], [0, 0xff, 0]);
    renderer.render_output(&mut server, MAIN).expect("frame");
    let main = renderer.snapshot_output(MAIN).expect("read back");
    assert_eq!(main.rgba[..3], [0, 0, 0xff]);
}

#[test]
fn the_desktop_and_menu_bar_run_once_per_display_and_follow_the_displays() {
    let mut server = two_displays();
    let shell = std::path::Path::new("/bin/true");
    server.start_client_on_every_display(shell);
    server.keep_session_clients_running();
    assert_eq!(
        server.display_clients(shell),
        [(MAIN.to_owned(), false), (SIDE.to_owned(), true)]
    );

    let arranged = config(
        &server,
        "arrangement {\n    display \"SIDE\" x=0 y=0 main=#true\n    display \"MAIN\" x=320 y=0\n}\n",
    );
    server.set_display_config(arranged);
    server.keep_session_clients_running();
    let mut clients = server.display_clients(shell);
    clients.sort();
    assert_eq!(clients, [(MAIN.to_owned(), true), (SIDE.to_owned(), false)]);

    server.remove_output(SIDE);
    server.keep_session_clients_running();
    assert_eq!(server.display_clients(shell), [(MAIN.to_owned(), false)]);
}

#[test]
fn a_rotated_display_is_laid_out_upright_and_drawn_turned() {
    let mut server = two_displays();
    let side = server.output_identity(SIDE).expect("side identity");
    let source = format!("display \"{side}\" {{\n    rotation 90\n}}\n");
    server.set_display_config(Config::parse(&source).expect("valid config"));
    assert_eq!(server.output_rect(SIDE), Some(rect(256, 0, 240, 320)));
    server.move_pointer_to(at(300.0, 100.0), 0);
    let mut client = TestClient::connect(&mut server);
    let pixels = quadrants(WINDOW.0 as usize, WINDOW.1 as usize, WINDOW.0 as usize * 4);
    client.show_window(
        &mut server,
        &ShmImage {
            width: WINDOW.0,
            height: WINDOW.1,
            stride: WINDOW.0 * 4,
            format: Format::Xrgb8888,
            bytes: &pixels,
        },
    );
    let mut renderer = Renderer::new().expect("renderer");

    renderer.render_output(&mut server, SIDE).expect("frame");
    let snapshot = renderer.snapshot_output(SIDE).expect("read back");

    assert_eq!(snapshot.size, (320, 240).into());
    let bounds = |color: [u8; 3]| {
        let points: Vec<(i32, i32)> = snapshot
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, pixel)| pixel[..3] == color)
            .map(|(at, _)| (at as i32 % 320, at as i32 / 320))
            .collect();
        let xs = points.iter().map(|(x, _)| *x);
        let ys = points.iter().map(|(_, y)| *y);
        (
            xs.clone().min().expect("drawn"),
            ys.clone().min().expect("drawn"),
            xs.max().expect("drawn"),
            ys.max().expect("drawn"),
        )
    };
    let red = bounds([0xff, 0, 0]);
    let green = bounds([0, 0xff, 0]);
    assert_eq!(
        (red.2 - red.0 + 1, red.3 - red.1 + 1),
        (WINDOW.1 / 2, WINDOW.0 / 2)
    );
    assert_eq!(red.0, green.0);
    assert_eq!(green.3 + 1, red.1);
}

#[test]
fn zoom_fills_the_display_the_window_is_on() {
    let mut server = two_displays();
    server.move_pointer_to(at(400.0, 100.0), 0);
    let mut app = app(&mut server);

    chord(&mut server, &[KEY_LEFTCTRL, KEY_LEFTMETA], KEY_UP);
    roundtrip(&mut server, &mut [&mut app]);
    app.fill_configured_size(&mut server, 0, RED);
    roundtrip(&mut server, &mut [&mut app]);

    assert_eq!(geometry(&server), rect(256, 0, 320, 240));
}

fn red_image(pixels: &[u8]) -> ShmImage<'_> {
    ShmImage {
        width: WINDOW.0,
        height: WINDOW.1,
        stride: WINDOW.0 * 4,
        format: Format::Xrgb8888,
        bytes: pixels,
    }
}

#[test]
fn an_apps_windows_cascade_only_from_its_windows_on_the_same_display() {
    let mut server = two_displays();
    let pixels = RED.repeat((WINDOW.0 * WINDOW.1) as usize);
    let image = red_image(&pixels);
    let mut terminal = TestClient::connect(&mut server);
    terminal.show_window_of(&mut server, &image, "org.example.Terminal");
    server.move_pointer_to(at(400.0, 100.0), 0);

    terminal.show_window_of(&mut server, &image, "org.example.Terminal");
    terminal.show_window_of(&mut server, &image, "org.example.Terminal");

    let geometries: Vec<Rect> = server
        .windows()
        .iter()
        .map(|window| window.geometry().expect("mapped"))
        .collect();
    assert_eq!(
        geometries,
        [
            rect(96, 72, 64, 48),
            rect(256 + 128, 96, 64, 48),
            rect(256 + 128 + 28, 96 + 28, 64, 48),
        ]
    );
    assert_eq!(server.output_of(&server.windows()[1]), SIDE);
    assert_eq!(server.output_of(&server.windows()[2]), SIDE);
}

#[test]
fn a_window_with_our_title_bar_moves_to_a_display_of_another_scale_with_its_bar() {
    const DECORATED: (i32, i32) = (120, 40);
    const BAR: i32 = 28;
    let mut server = two_displays();
    server.set_reduce_motion(true);
    let mut settings = server.output_settings(SIDE).expect("side display");
    settings.scale = 2.0;
    server.configure_output(SIDE, settings);
    let mut app = TestClient::connect(&mut server);
    app.use_seat(&mut server);
    let pixels = RED.repeat((DECORATED.0 * DECORATED.1) as usize);
    let image = ShmImage {
        width: DECORATED.0,
        height: DECORATED.1,
        stride: DECORATED.0 * 4,
        format: Format::Xrgb8888,
        bytes: &pixels,
    };
    app.show_decorated_window(&mut server, &image, Some(DecorationMode::ServerSide));
    app.exchange(&mut server);
    app.commit_buffer(0, &image, (0, 0, DECORATED.0, DECORATED.1));
    app.exchange(&mut server);
    let decorated = (DECORATED.0, DECORATED.1 + BAR);
    assert_eq!(geometry(&server), rect(68, 76, decorated.0, decorated.1));

    chord(
        &mut server,
        &[KEY_LEFTALT, KEY_LEFTMETA, KEY_LEFTSHIFT],
        KEY_RIGHT,
    );
    roundtrip(&mut server, &mut [&mut app]);

    assert_eq!(server.output_of(&server.windows()[0]), SIDE);
    assert_eq!(geometry(&server), rect(276, 35, decorated.0, decorated.1));
    assert_eq!(app.preferred_scale(&mut server, 0), Some(240));
    let mut renderer = Renderer::new().expect("renderer");
    renderer.render_output(&mut server, SIDE).expect("frame");
    let snapshot = renderer.snapshot_output(SIDE).expect("read back");
    let pixel = |x: i32, y: i32| {
        let at = (((35 + y) * 2 * snapshot.size.w + (276 - 256 + x) * 2) * 4) as usize;
        [
            snapshot.rgba[at],
            snapshot.rgba[at + 1],
            snapshot.rgba[at + 2],
        ]
    };
    assert!(pixel(100, BAR - 3).iter().all(|channel| *channel > 0xf0));
    assert_eq!(pixel(60, BAR + 3), [0xff, 0, 0]);

    chord(
        &mut server,
        &[KEY_LEFTALT, KEY_LEFTMETA, KEY_LEFTSHIFT],
        KEY_LEFT,
    );
    roundtrip(&mut server, &mut [&mut app]);

    assert_eq!(server.output_of(&server.windows()[0]), MAIN);
    assert_eq!(geometry(&server), rect(68, 76, decorated.0, decorated.1));
}
