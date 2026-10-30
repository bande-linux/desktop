#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use smithay::utils::{Point, Size};
use unbound_compositor::{Headless, Renderer, automatic_scale};
use unbound_test_wayland::{
    Anchor, BLUE, BrightnessKey, GREEN, KeyboardInteractivity, Layer, LayerRequest, RED, ShmImage,
    TestClient, VolumeKey, roundtrip,
};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (256, 192);
const BTN_LEFT: u32 = 0x110;

fn server() -> Headless {
    Headless::new(Size::from(OUTPUT)).expect("headless compositor")
}

fn red(width: i32, height: i32) -> Vec<u8> {
    RED.repeat((width * height) as usize)
}

fn image(pixels: &[u8], width: i32, height: i32) -> ShmImage<'_> {
    ShmImage {
        width,
        height,
        stride: width * 4,
        format: Format::Xrgb8888,
        bytes: pixels,
    }
}

fn panel(layer: Layer, anchor: Anchor, size: (u32, u32), exclusive_zone: i32) -> LayerRequest {
    LayerRequest {
        layer,
        anchor,
        size,
        exclusive_zone,
        keyboard: KeyboardInteractivity::None,
        color: GREEN,
    }
}

fn pixel(renderer: &mut Renderer, server: &mut Headless, x: i32, y: i32) -> [u8; 3] {
    renderer.render(server).expect("frame");
    let snapshot = renderer.snapshot(server).expect("read back");
    let at = ((y * snapshot.size.w + x) * 4) as usize;
    [
        snapshot.rgba[at],
        snapshot.rgba[at + 1],
        snapshot.rgba[at + 2],
    ]
}

fn rgb(bgra: [u8; 4]) -> [u8; 3] {
    [bgra[2], bgra[1], bgra[0]]
}

#[test]
fn a_left_panel_reserves_its_exclusive_zone_and_windows_are_centered_beside_it() {
    let mut server = server();
    let mut shell = TestClient::connect(&mut server);
    let size = shell.show_layer(
        &mut server,
        &panel(
            Layer::Top,
            Anchor::Left | Anchor::Top | Anchor::Bottom,
            (40, 0),
            40,
        ),
    );
    assert_eq!(size, (40, OUTPUT.1 as u32));

    let mut app = TestClient::connect(&mut server);
    let pixels = red(96, 64);
    app.show_window(&mut server, &image(&pixels, 96, 64));

    assert_eq!(
        server.windows()[0].location(),
        Point::from((40 + (216 - 96) / 2, (192 - 64) / 2))
    );
}

#[test]
fn a_top_bar_pushes_windows_below_it() {
    let mut server = server();
    let mut shell = TestClient::connect(&mut server);
    let size = shell.show_layer(
        &mut server,
        &panel(
            Layer::Top,
            Anchor::Top | Anchor::Left | Anchor::Right,
            (0, 24),
            24,
        ),
    );
    assert_eq!(size, (OUTPUT.0 as u32, 24));

    let mut app = TestClient::connect(&mut server);
    let pixels = red(96, 64);
    app.show_window(&mut server, &image(&pixels, 96, 64));

    assert_eq!(
        server.windows()[0].location(),
        Point::from(((256 - 96) / 2, 24 + (168 - 64) / 2))
    );
}

#[test]
fn the_menu_bar_spans_the_whole_top_edge_even_when_the_dock_reserved_its_side_first() {
    let mut server = server();
    let mut shell = TestClient::connect(&mut server);
    shell.show_named_layer(
        &mut server,
        &panel(Layer::Top, Anchor::Left, (40, 120), 40),
        "dock",
    );
    let bar = shell.show_named_layer(
        &mut server,
        &panel(
            Layer::Top,
            Anchor::Top | Anchor::Left | Anchor::Right,
            (0, 24),
            24,
        ),
        "menubar",
    );

    assert_eq!(bar, (OUTPUT.0 as u32, 24));
}

#[test]
fn background_layers_are_drawn_below_windows_and_top_layers_above() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut app = TestClient::connect(&mut server);
    let pixels = red(32, 32);
    app.show_window(&mut server, &image(&pixels, 32, 32));
    let mut shell = TestClient::connect(&mut server);
    let everywhere = Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right;
    shell.show_layer(
        &mut server,
        &panel(Layer::Background, everywhere, (0, 0), 0),
    );
    let center = (OUTPUT.0 / 2, OUTPUT.1 / 2);

    assert_eq!(
        pixel(&mut renderer, &mut server, center.0, center.1),
        rgb(RED)
    );
    assert_eq!(pixel(&mut renderer, &mut server, 2, 2), rgb(GREEN));

    shell.show_layer(
        &mut server,
        &LayerRequest {
            color: BLUE,
            ..panel(Layer::Overlay, everywhere, (0, 0), 0)
        },
    );

    assert_eq!(
        pixel(&mut renderer, &mut server, center.0, center.1),
        rgb(BLUE)
    );
}

#[test]
fn clicking_a_panel_keeps_the_focused_window_focused() {
    let mut server = server();
    let mut shell = TestClient::connect(&mut server);
    shell.show_layer(
        &mut server,
        &panel(
            Layer::Top,
            Anchor::Left | Anchor::Top | Anchor::Bottom,
            (40, 0),
            40,
        ),
    );
    let mut app = TestClient::connect(&mut server);
    app.use_seat(&mut server);
    let pixels = red(32, 32);
    app.show_window(&mut server, &image(&pixels, 32, 32));
    assert!(app.keyboard_focus_on(0));

    server.move_pointer_to(Point::from((10.0, 10.0)), 1);
    server.press_button(BTN_LEFT, true, 2);
    server.press_button(BTN_LEFT, false, 3);
    roundtrip(&mut server, &mut [&mut shell, &mut app]);

    assert!(app.keyboard_focus_on(0));
}

#[test]
fn the_focused_app_can_hand_activation_to_another_app() {
    let mut server = server();
    let mut other = TestClient::connect(&mut server);
    other.use_seat(&mut server);
    let small = red(32, 32);
    other.show_window(&mut server, &image(&small, 32, 32));
    let mut focused = TestClient::connect(&mut server);
    focused.use_seat(&mut server);
    let large = red(96, 96);
    focused.show_window(&mut server, &image(&large, 96, 96));
    roundtrip(&mut server, &mut [&mut other, &mut focused]);
    assert!(focused.keyboard_focus_on(0));

    let own_token = other.activation_token(&mut server);
    other.activate(&mut server, 0, &own_token);
    roundtrip(&mut server, &mut [&mut other, &mut focused]);
    assert!(focused.keyboard_focus_on(0));

    let handed = focused.activation_token(&mut server);
    other.activate(&mut server, 0, &handed);
    roundtrip(&mut server, &mut [&mut other, &mut focused]);
    assert!(other.keyboard_focus_on(0));
    assert!(!focused.keyboard_focus_on(0));
}

#[test]
fn a_new_window_activated_with_a_handed_token_takes_focus() {
    let mut server = server();
    let mut other = TestClient::connect(&mut server);
    other.use_seat(&mut server);
    let small = red(32, 32);
    other.show_window(&mut server, &image(&small, 32, 32));
    let mut focused = TestClient::connect(&mut server);
    focused.use_seat(&mut server);
    let large = red(96, 96);
    focused.show_window(&mut server, &image(&large, 96, 96));
    roundtrip(&mut server, &mut [&mut other, &mut focused]);

    let handed = focused.activation_token(&mut server);
    other.show_window_activated(&mut server, &image(&small, 32, 32), &handed);
    roundtrip(&mut server, &mut [&mut other, &mut focused]);

    assert!(other.keyboard_focus_on(1));
}

#[test]
fn a_viewport_destination_sets_the_window_size() {
    let mut server = server();
    let mut app = TestClient::connect(&mut server);
    let pixels = red(32, 32);
    app.show_window_scaled(&mut server, &image(&pixels, 32, 32), (64, 48));

    let [window] = server.windows() else {
        panic!("one window");
    };
    assert_eq!(window.size(), Some(Size::from((64, 48))));
    assert_eq!(
        window.location(),
        Point::from(((256 - 64) / 2, (192 - 48) / 2))
    );
}

#[test]
fn surfaces_are_told_the_output_scale_and_told_again_when_it_changes() {
    let mut server = server();
    let mut app = TestClient::connect(&mut server);
    let pixels = red(32, 32);
    app.show_window(&mut server, &image(&pixels, 32, 32));
    assert_eq!(app.preferred_scale(&mut server, 0), Some(120));

    server.set_output_scale(2.0);
    app.exchange(&mut server);

    assert_eq!(app.preferred_scale(&mut server, 0), Some(240));
}

#[test]
fn on_a_2x_output_a_window_is_laid_out_in_points_and_drawn_in_pixels() {
    let mut server = server();
    server.set_output_scale(2.0);
    assert_eq!(server.logical_output_size(), Size::from((128, 96)));
    let mut app = TestClient::connect(&mut server);
    let pixels = red(64, 64);
    app.show_window_scaled(&mut server, &image(&pixels, 64, 64), (32, 32));

    let [window] = server.windows() else {
        panic!("one window");
    };
    assert_eq!(window.size(), Some(Size::from((32, 32))));
    assert_eq!(
        window.location(),
        Point::from(((128 - 32) / 2, (96 - 32) / 2))
    );

    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let (left, top) = (2 * (128 - 32) / 2, 2 * (96 - 32) / 2);
    assert_eq!(
        pixel(&mut renderer, &mut server, left + 32, top + 1),
        rgb(RED)
    );
    assert_eq!(
        pixel(&mut renderer, &mut server, left + 62, top + 32),
        rgb(RED)
    );
    assert_ne!(
        pixel(&mut renderer, &mut server, left + 65, top + 30),
        rgb(RED)
    );
}

#[test]
fn a_scale_follows_the_displays_pixel_density() {
    let mm = |w, h| Size::from((w, h));
    let px = |w, h| Size::from((w, h));
    assert_eq!(automatic_scale(px(1920, 1080), mm(527, 296)), 1.0);
    assert_eq!(automatic_scale(px(3024, 1964), mm(302, 196)), 2.25);
    assert_eq!(automatic_scale(px(5120, 2880), mm(597, 336)), 2.0);
    assert_eq!(automatic_scale(px(2904, 1818), mm(0, 0)), 2.0);
    assert_eq!(automatic_scale(px(1920, 1080), mm(0, 0)), 1.0);
}

const KEY_SPACE: u32 = 57;
const KEY_LEFTMETA: u32 = 125;
const KEY_A: u32 = 30;
const KEY_3: u32 = 4;
const KEY_4: u32 = 5;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_LEFTALT: u32 = 56;
const KEY_MUTE: u32 = 113;
const KEY_VOLUMEUP: u32 = 115;
const KEY_BRIGHTNESSDOWN: u32 = 224;
const KEY_KBDILLUMUP: u32 = 230;

fn launcher() -> LayerRequest {
    LayerRequest {
        keyboard: KeyboardInteractivity::Exclusive,
        ..panel(
            Layer::Overlay,
            Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
            (0, 0),
            0,
        )
    }
}

fn press(server: &mut Headless, keys: &[u32]) {
    for key in keys {
        server.key(*key, true, 0);
    }
    for key in keys.iter().rev() {
        server.key(*key, false, 0);
    }
}

#[test]
fn super_space_asks_the_shell_for_the_launcher_and_is_not_delivered_to_the_app() {
    let mut server = server();
    let mut app = TestClient::connect(&mut server);
    app.use_seat(&mut server);
    let pixels = red(32, 32);
    app.show_window(&mut server, &image(&pixels, 32, 32));
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_shell(&mut server);

    press(&mut server, &[KEY_LEFTMETA, KEY_SPACE]);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);

    assert_eq!(shell.launcher_requests(), 1);
    assert!(
        app.keys().iter().all(|(key, _)| *key != KEY_SPACE),
        "the app saw {:?}",
        app.keys()
    );
}

#[test]
fn super_shift_3_asks_the_shell_for_a_screen_capture_and_is_not_delivered_to_the_app() {
    let mut server = server();
    let mut app = TestClient::connect(&mut server);
    app.use_seat(&mut server);
    let pixels = red(32, 32);
    app.show_window(&mut server, &image(&pixels, 32, 32));
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_shell(&mut server);

    press(&mut server, &[KEY_LEFTMETA, KEY_3]);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);
    assert_eq!(shell.screen_capture_requests(), 0);
    let threes = |app: &TestClient| app.keys().iter().filter(|(key, _)| *key == KEY_3).count();
    let seen = threes(&app);

    press(&mut server, &[KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_3]);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);

    assert_eq!(shell.screen_capture_requests(), 1);
    assert_eq!(shell.launcher_requests(), 0);
    assert_eq!(threes(&app), seen, "the app saw {:?}", app.keys());
}

#[test]
fn super_shift_4_asks_the_shell_for_an_area_capture() {
    let mut server = server();
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_shell(&mut server);

    press(&mut server, &[KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_4]);
    roundtrip(&mut server, &mut [&mut shell]);

    assert_eq!(shell.area_capture_requests(), 1);
    assert_eq!(shell.screen_capture_requests(), 0);
}

#[test]
fn the_shell_protocol_and_the_window_list_are_hidden_from_apps() {
    let mut server = server();
    let app = TestClient::connect(&mut server);
    let shell = TestClient::connect_as_shell(&mut server);

    assert!(!app.advertises("unbound_shell_v1"));
    assert!(!app.advertises("ext_foreign_toplevel_list_v1"));
    assert!(shell.advertises("unbound_shell_v1"));
    assert!(shell.advertises("ext_foreign_toplevel_list_v1"));
}

#[test]
fn an_exclusive_overlay_takes_the_keyboard_and_returns_it_when_hidden() {
    let mut server = server();
    let mut app = TestClient::connect(&mut server);
    app.use_seat(&mut server);
    let pixels = red(32, 32);
    app.show_window(&mut server, &image(&pixels, 32, 32));
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.use_seat(&mut server);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);
    assert!(app.keyboard_focus_on(0));

    shell.show_layer(&mut server, &launcher());
    press(&mut server, &[KEY_A]);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);

    assert!(!app.keyboard_focus_on(0));
    assert_eq!(shell.keys(), [(KEY_A, true), (KEY_A, false)]);
    assert!(app.keys().is_empty());

    shell.hide_layer(&mut server, 0);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);

    assert!(app.keyboard_focus_on(0));

    assert!(
        shell.reconfigure_layer(&mut server, 0),
        "a layer committed without a buffer after unmapping is configured again"
    );
}

fn exclusive(layer: Layer) -> LayerRequest {
    LayerRequest {
        layer,
        ..launcher()
    }
}

fn typed_a(server: &mut Headless, clients: &mut [&mut TestClient]) {
    press(server, &[KEY_A]);
    roundtrip(server, clients);
}

#[test]
fn the_keyboard_returns_to_an_exclusive_overlay_when_one_above_it_is_hidden() {
    let mut server = server();
    let mut app = TestClient::connect(&mut server);
    app.use_seat(&mut server);
    let pixels = red(32, 32);
    app.show_window(&mut server, &image(&pixels, 32, 32));
    let mut lower = TestClient::connect_as_shell(&mut server);
    lower.use_seat(&mut server);
    let mut upper = TestClient::connect_as_shell(&mut server);
    upper.use_seat(&mut server);
    lower.show_layer(&mut server, &exclusive(Layer::Overlay));
    upper.show_layer(&mut server, &exclusive(Layer::Overlay));

    upper.hide_layer(&mut server, 0);
    typed_a(&mut server, &mut [&mut app, &mut lower, &mut upper]);

    assert!(app.keys().is_empty());
    assert!(upper.keys().is_empty());
    assert_eq!(lower.keys(), [(KEY_A, true), (KEY_A, false)]);

    lower.hide_layer(&mut server, 0);
    roundtrip(&mut server, &mut [&mut app, &mut lower, &mut upper]);
    assert!(app.keyboard_focus_on(0));
}

#[test]
fn an_exclusive_overlay_keeps_the_keyboard_from_an_exclusive_top_layer() {
    let mut server = server();
    let mut app = TestClient::connect(&mut server);
    app.use_seat(&mut server);
    let pixels = red(32, 32);
    app.show_window(&mut server, &image(&pixels, 32, 32));
    let mut overlay = TestClient::connect_as_shell(&mut server);
    overlay.use_seat(&mut server);
    let mut top = TestClient::connect_as_shell(&mut server);
    top.use_seat(&mut server);
    overlay.show_layer(&mut server, &exclusive(Layer::Overlay));

    top.show_layer(&mut server, &exclusive(Layer::Top));
    top.request_layer_frame(&mut server, 0);
    typed_a(&mut server, &mut [&mut app, &mut overlay, &mut top]);

    assert!(top.keys().is_empty());
    assert_eq!(overlay.keys(), [(KEY_A, true), (KEY_A, false)]);

    overlay.close_layer(&mut server, 0);
    typed_a(&mut server, &mut [&mut app, &mut overlay, &mut top]);

    assert_eq!(top.keys(), [(KEY_A, true), (KEY_A, false)]);
    assert!(app.keys().is_empty());
}

#[test]
fn a_click_on_a_window_under_an_exclusive_overlay_does_not_take_the_keyboard() {
    let mut server = server();
    let mut app = TestClient::connect(&mut server);
    app.use_seat(&mut server);
    let pixels = red(32, 32);
    app.show_window(&mut server, &image(&pixels, 32, 32));
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.use_seat(&mut server);
    shell.show_layer(&mut server, &launcher());
    let window = server.windows()[0].location();

    server.move_pointer_to(
        Point::from((f64::from(window.x + 4), f64::from(window.y + 4))),
        0,
    );
    server.press_button(BTN_LEFT, true, 0);
    server.press_button(BTN_LEFT, false, 0);
    press(&mut server, &[KEY_A]);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);

    assert!(app.keys().is_empty());
    assert_eq!(shell.keys(), [(KEY_A, true), (KEY_A, false)]);

    shell.close_layer(&mut server, 0);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);
    assert!(app.keyboard_focus_on(0));
}

#[test]
fn the_shell_sees_running_apps_and_brings_one_forward() {
    let mut server = server();
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_shell(&mut server);
    let mut editor = TestClient::connect(&mut server);
    editor.use_seat(&mut server);
    let pixels = red(32, 32);
    editor.show_window(&mut server, &image(&pixels, 32, 32));
    editor.set_app_id(&mut server, 0, "org.example.Editor");
    let mut terminal = TestClient::connect(&mut server);
    terminal.use_seat(&mut server);
    terminal.show_window(&mut server, &image(&pixels, 32, 32));
    terminal.set_app_id(&mut server, 0, "org.unbound.Terminal");
    let token = terminal.activation_token(&mut server);
    terminal.activate(&mut server, 0, &token);
    roundtrip(&mut server, &mut [&mut shell, &mut editor, &mut terminal]);
    assert!(terminal.keyboard_focus_on(0));

    let mut running = shell.running_app_ids();
    running.sort();
    assert_eq!(running, ["org.example.Editor", "org.unbound.Terminal"]);

    shell.activate_app(&mut server, "org.example.Editor");
    roundtrip(&mut server, &mut [&mut shell, &mut editor, &mut terminal]);

    assert!(editor.keyboard_focus_on(0));

    editor.close_window(&mut server, 0);
    roundtrip(&mut server, &mut [&mut shell, &mut editor, &mut terminal]);
    assert_eq!(shell.running_app_ids(), ["org.unbound.Terminal"]);
}

#[test]
fn the_shell_can_hand_activation_to_an_app_without_holding_the_keyboard() {
    let mut server = server();
    let pixels = red(32, 32);
    let mut launched = TestClient::connect(&mut server);
    launched.use_seat(&mut server);
    launched.show_window(&mut server, &image(&pixels, 32, 32));
    let mut app = TestClient::connect(&mut server);
    app.use_seat(&mut server);
    app.show_window(&mut server, &image(&pixels, 32, 32));
    roundtrip(&mut server, &mut [&mut app, &mut launched]);
    assert!(app.keyboard_focus_on(0));
    let mut shell = TestClient::connect_as_shell(&mut server);
    let token = shell.activation_token(&mut server);

    launched.show_window_activated(&mut server, &image(&pixels, 32, 32), &token);
    roundtrip(&mut server, &mut [&mut app, &mut launched]);

    assert!(launched.keyboard_focus_on(1));
}

#[test]
fn layer_surfaces_receive_frame_callbacks() {
    let mut server = server();
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.show_layer(
        &mut server,
        &panel(
            Layer::Top,
            Anchor::Left | Anchor::Top | Anchor::Bottom,
            (40, 0),
            40,
        ),
    );
    shell.request_layer_frame(&mut server, 0);

    server.latch();
    server
        .present(unbound_compositor::Vblank {
            time: std::time::Duration::from_millis(8),
            sequence: 1,
        })
        .expect("present");
    shell.exchange(&mut server);

    assert_eq!(shell.frames_done(), 1);
}

#[test]
fn wheel_scrolling_reaches_the_window_under_the_pointer() {
    let mut server = server();
    let mut app = TestClient::connect(&mut server);
    app.use_seat(&mut server);
    let pixels = red(32, 32);
    app.show_window(&mut server, &image(&pixels, 32, 32));
    let window = server.windows()[0].location();
    server.move_pointer_to(
        Point::from((f64::from(window.x + 4), f64::from(window.y + 4))),
        0,
    );

    server.scroll(
        smithay::input::pointer::AxisFrame::new(0)
            .source(smithay::backend::input::AxisSource::Wheel)
            .value(smithay::backend::input::Axis::Vertical, 30.0),
    );
    app.exchange(&mut server);

    assert_eq!(app.scrolled(), 30.0);
}

#[test]
fn the_shell_hears_which_app_is_active() {
    let mut server = server();
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_shell(&mut server);
    assert_eq!(shell.active_apps(), [""]);

    let mut app = TestClient::connect(&mut server);
    app.use_seat(&mut server);
    let pixels = red(32, 32);
    app.show_window(&mut server, &image(&pixels, 32, 32));
    app.set_app_id(&mut server, 0, "org.example.Editor");
    let mut other = TestClient::connect(&mut server);
    other.use_seat(&mut server);
    other.show_window(&mut server, &image(&pixels, 32, 32));
    other.set_app_id(&mut server, 0, "org.example.Viewer");
    roundtrip(&mut server, &mut [&mut app, &mut other, &mut shell]);
    let heard = shell.active_apps().to_vec();

    assert_eq!(heard[..2], ["", "org.example.Editor"]);
    assert_eq!(heard.last().map(String::as_str), Some("org.example.Viewer"));
}

#[test]
fn an_app_stays_active_after_closing_its_last_window_until_it_quits() {
    let mut server = server();
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_shell(&mut server);
    let pixels = red(32, 32);
    let mut other = TestClient::connect(&mut server);
    other.use_seat(&mut server);
    other.show_window(&mut server, &image(&pixels, 32, 32));
    other.set_app_id(&mut server, 0, "org.example.Viewer");
    let mut app = TestClient::connect(&mut server);
    app.use_seat(&mut server);
    app.show_window(&mut server, &image(&pixels, 32, 32));
    app.set_app_id(&mut server, 0, "org.example.Editor");
    roundtrip(&mut server, &mut [&mut app, &mut other, &mut shell]);

    app.close_window(&mut server, 0);
    roundtrip(&mut server, &mut [&mut app, &mut other, &mut shell]);
    assert_eq!(
        shell.active_apps().last().map(String::as_str),
        Some("org.example.Editor")
    );
    drop(app);
    roundtrip(&mut server, &mut [&mut other, &mut shell]);

    assert_eq!(
        shell.active_apps().last().map(String::as_str),
        Some("org.example.Viewer")
    );
}

#[test]
fn volume_keys_go_to_the_shell_without_super_and_option_shift_makes_them_fine() {
    let mut server = server();
    let mut app = TestClient::connect(&mut server);
    app.use_seat(&mut server);
    let pixels = red(32, 32);
    app.show_window(&mut server, &image(&pixels, 32, 32));
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_shell(&mut server);

    press(&mut server, &[KEY_MUTE]);
    press(&mut server, &[KEY_LEFTALT, KEY_LEFTSHIFT, KEY_VOLUMEUP]);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);

    assert_eq!(
        shell.volume_keys(),
        [(VolumeKey::Mute, false), (VolumeKey::Up, true)]
    );
    assert!(
        app.keys()
            .iter()
            .all(|(key, _)| *key != KEY_MUTE && *key != KEY_VOLUMEUP),
        "the app saw {:?}",
        app.keys()
    );
}

#[test]
fn brightness_keys_go_to_the_shell_and_option_shift_makes_them_fine() {
    let mut server = server();
    let mut app = TestClient::connect(&mut server);
    app.use_seat(&mut server);
    let pixels = red(32, 32);
    app.show_window(&mut server, &image(&pixels, 32, 32));
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_shell(&mut server);

    press(&mut server, &[KEY_BRIGHTNESSDOWN]);
    press(&mut server, &[KEY_LEFTALT, KEY_LEFTSHIFT, KEY_KBDILLUMUP]);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);

    assert_eq!(
        shell.brightness_keys(),
        [
            (BrightnessKey::DisplayDown, false),
            (BrightnessKey::KeyboardUp, true)
        ]
    );
    assert!(
        app.keys()
            .iter()
            .all(|(key, _)| *key != KEY_BRIGHTNESSDOWN && *key != KEY_KBDILLUMUP),
        "the app saw {:?}",
        app.keys()
    );
}

#[test]
fn the_menu_bar_toggles_control_center_through_the_compositor() {
    let mut server = server();
    let mut menu_bar = TestClient::connect_as_shell(&mut server);
    menu_bar.join_shell(&mut server);
    let mut control_center = TestClient::connect_as_shell(&mut server);
    control_center.join_shell(&mut server);

    menu_bar.toggle_control_center(&mut server);
    roundtrip(&mut server, &mut [&mut menu_bar, &mut control_center]);

    assert_eq!(control_center.control_center_toggles(), 1);
}
