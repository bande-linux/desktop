#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use smithay::utils::{Point, Size};
use unbound_compositor::{Headless, Rect, Renderer};
use unbound_test_wayland::{
    Anchor, BLUE, GREEN, KeyboardInteractivity, Layer, LayerRequest, PopupRequest, RED, ResizeEdge,
    ShmImage, TestClient, ToplevelState, roundtrip,
};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (256, 192);
const WINDOW: (i32, i32) = (64, 48);
const CENTERED: (i32, i32) = (96, 72);
const BAR: i32 = 24;
const BTN_LEFT: u32 = 0x110;
const WHITE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];

fn server() -> Headless {
    Headless::new(Size::from(OUTPUT)).expect("headless compositor")
}

fn app(server: &mut Headless, color: [u8; 4]) -> TestClient {
    let mut client = TestClient::connect(server);
    client.use_seat(server);
    let pixels = color.repeat((WINDOW.0 * WINDOW.1) as usize);
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

fn top_bar(server: &mut Headless) -> TestClient {
    let mut shell = TestClient::connect_as_shell(server);
    shell.show_layer(
        server,
        &LayerRequest {
            layer: Layer::Top,
            anchor: Anchor::Top | Anchor::Left | Anchor::Right,
            size: (0, BAR as u32),
            exclusive_zone: BAR,
            keyboard: KeyboardInteractivity::None,
            color: GREEN,
        },
    );
    shell
}

fn geometry(server: &Headless) -> Rect {
    server.windows()[0].geometry().expect("a mapped window")
}

fn at(x: i32, y: i32) -> Point<f64, smithay::utils::Logical> {
    (f64::from(x), f64::from(y)).into()
}

fn click(server: &mut Headless, x: i32, y: i32) {
    server.move_pointer_to(at(x, y), 0);
    server.press_button(BTN_LEFT, true, 0);
    server.press_button(BTN_LEFT, false, 0);
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

fn drag_with(
    server: &mut Headless,
    client: &mut TestClient,
    from: (i32, i32),
    by: (i32, i32),
    request: impl FnOnce(&mut Headless, &mut TestClient),
) {
    server.move_pointer_to(at(from.0, from.1), 0);
    server.press_button(BTN_LEFT, true, 0);
    roundtrip(server, &mut [client]);
    request(server, client);
    server.move_pointer_to(at(from.0 + by.0, from.1 + by.1), 1);
    roundtrip(server, &mut [client]);
}

#[test]
fn a_moved_window_follows_the_pointer_and_stays_put_after_release() {
    let mut server = server();
    let mut editor = app(&mut server, RED);
    assert_eq!(geometry(&server).location, CENTERED.into());

    drag_with(
        &mut server,
        &mut editor,
        (110, 80),
        (20, 30),
        |server, client| client.request_move(server, 0),
    );
    assert_eq!(geometry(&server).location, (116, 102).into());

    server.press_button(BTN_LEFT, false, 2);
    server.move_pointer_to(at(10, 10), 3);
    roundtrip(&mut server, &mut [&mut editor]);
    assert_eq!(geometry(&server).location, (116, 102).into());
}

#[test]
fn a_window_cannot_be_dragged_under_the_top_bar() {
    let mut server = server();
    let mut shell = top_bar(&mut server);
    let mut editor = app(&mut server, RED);
    roundtrip(&mut server, &mut [&mut shell, &mut editor]);
    let start = geometry(&server).location;

    drag_with(
        &mut server,
        &mut editor,
        (start.x + 10, start.y + 5),
        (0, -150),
        |server, client| client.request_move(server, 0),
    );

    assert_eq!(geometry(&server).location, (start.x, BAR).into());
}

#[test]
fn a_move_without_a_held_button_is_ignored() {
    let mut server = server();
    let mut editor = app(&mut server, RED);
    click(&mut server, 110, 80);
    roundtrip(&mut server, &mut [&mut editor]);

    editor.request_move(&mut server, 0);
    server.move_pointer_to(at(150, 150), 1);
    roundtrip(&mut server, &mut [&mut editor]);

    assert_eq!(geometry(&server).location, CENTERED.into());
}

#[test]
fn resizing_from_the_bottom_right_corner_grows_the_window_toward_the_pointer() {
    let mut server = server();
    let mut editor = app(&mut server, RED);

    drag_with(
        &mut server,
        &mut editor,
        (158, 118),
        (20, 10),
        |server, client| client.request_resize(server, 0, ResizeEdge::BottomRight),
    );
    let configured = editor.configured(0).expect("a configure");
    assert_eq!((configured.width, configured.height), (84, 58));
    assert!(configured.states.contains(&ToplevelState::Resizing));
    editor.fill_configured_size(&mut server, 0, RED);
    assert_eq!(
        geometry(&server),
        Rect {
            location: CENTERED.into(),
            size: (84, 58).into(),
        }
    );

    server.press_button(BTN_LEFT, false, 2);
    roundtrip(&mut server, &mut [&mut editor]);
    let configured = editor.configured(0).expect("a configure");
    assert!(!configured.states.contains(&ToplevelState::Resizing));
}

#[test]
fn resizing_from_the_left_edge_keeps_the_right_edge_in_place() {
    let mut server = server();
    let mut editor = app(&mut server, RED);
    let right = CENTERED.0 + WINDOW.0;

    drag_with(
        &mut server,
        &mut editor,
        (97, 90),
        (-20, 0),
        |server, client| client.request_resize(server, 0, ResizeEdge::Left),
    );
    editor.fill_configured_size(&mut server, 0, RED);

    let window = geometry(&server);
    assert_eq!(window.size, (84, 48).into());
    assert_eq!(window.location.x + window.size.w, right);
    assert_eq!(window.location.y, CENTERED.1);
}

#[test]
fn resizing_stops_at_the_minimum_size_the_app_sets() {
    let mut server = server();
    let mut editor = app(&mut server, RED);
    editor.set_min_size(&mut server, 0, 50, 40);

    drag_with(
        &mut server,
        &mut editor,
        (158, 118),
        (-40, -40),
        |server, client| client.request_resize(server, 0, ResizeEdge::BottomRight),
    );

    let configured = editor.configured(0).expect("a configure");
    assert_eq!((configured.width, configured.height), (50, 40));
}

#[test]
fn zoom_fills_the_area_beside_the_bar_and_unzoom_restores_the_window() {
    let mut server = server();
    let mut shell = top_bar(&mut server);
    let mut editor = app(&mut server, RED);
    roundtrip(&mut server, &mut [&mut shell, &mut editor]);
    let before = geometry(&server);

    editor.set_maximized(&mut server, 0, true);
    let configured = editor.configured(0).expect("a configure");
    assert_eq!(
        (configured.width, configured.height),
        (OUTPUT.0, OUTPUT.1 - BAR)
    );
    assert!(configured.states.contains(&ToplevelState::Maximized));
    editor.fill_configured_size(&mut server, 0, RED);
    assert_eq!(
        geometry(&server),
        Rect {
            location: (0, BAR).into(),
            size: (OUTPUT.0, OUTPUT.1 - BAR).into(),
        }
    );

    editor.set_maximized(&mut server, 0, false);
    let configured = editor.configured(0).expect("a configure");
    assert!(!configured.states.contains(&ToplevelState::Maximized));
    assert_eq!((configured.width, configured.height), WINDOW);
    editor.fill_configured_size(&mut server, 0, RED);
    assert_eq!(geometry(&server), before);
}

#[test]
fn a_fullscreen_window_covers_the_output_including_the_bar() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut shell = top_bar(&mut server);
    let mut editor = app(&mut server, RED);
    roundtrip(&mut server, &mut [&mut shell, &mut editor]);
    let before = geometry(&server);

    editor.set_fullscreen(&mut server, 0, true);
    let configured = editor.configured(0).expect("a configure");
    assert_eq!((configured.width, configured.height), OUTPUT);
    assert!(configured.states.contains(&ToplevelState::Fullscreen));
    editor.fill_configured_size(&mut server, 0, BLUE);
    assert_eq!(geometry(&server).location, (0, 0).into());
    assert_eq!(pixel(&mut renderer, &mut server, 10, 5), rgb(BLUE));

    editor.set_fullscreen(&mut server, 0, false);
    let configured = editor.configured(0).expect("a configure");
    assert!(!configured.states.contains(&ToplevelState::Fullscreen));
    editor.fill_configured_size(&mut server, 0, RED);
    assert_eq!(geometry(&server), before);
    assert_eq!(pixel(&mut renderer, &mut server, 10, 5), rgb(GREEN));
}

#[test]
fn only_the_key_window_is_told_it_is_activated() {
    let mut server = server();
    let mut first = app(&mut server, RED);
    let mut second = app(&mut server, BLUE);
    click(&mut server, 110, 80);
    roundtrip(&mut server, &mut [&mut first, &mut second]);

    let activated = |client: &TestClient| {
        client
            .configured(0)
            .expect("a configure")
            .states
            .contains(&ToplevelState::Activated)
    };
    let focused_first = first.keyboard_focus_on(0);
    assert_eq!(activated(&first), focused_first);
    assert_eq!(activated(&second), !focused_first);
}

#[test]
fn minimizing_hides_the_window_keeps_the_app_active_and_the_shell_brings_it_back() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_shell(&mut server);
    let mut behind = app(&mut server, RED);
    let mut front = app(&mut server, BLUE);
    front.set_app_id(&mut server, 0, "org.example.Front");
    shell.activate_app(&mut server, "org.example.Front");
    roundtrip(&mut server, &mut [&mut shell, &mut behind, &mut front]);
    assert!(front.keyboard_focus_on(0));
    assert_eq!(pixel(&mut renderer, &mut server, 110, 80), rgb(BLUE));

    front.set_minimized(&mut server, 0);
    roundtrip(&mut server, &mut [&mut shell, &mut behind, &mut front]);
    assert_eq!(pixel(&mut renderer, &mut server, 110, 80), rgb(RED));
    assert!(!front.keyboard_focus_on(0));
    assert!(!behind.keyboard_focus_on(0));

    shell.activate_app(&mut server, "org.example.Front");
    roundtrip(&mut server, &mut [&mut shell, &mut behind, &mut front]);
    assert_eq!(pixel(&mut renderer, &mut server, 110, 80), rgb(BLUE));
    assert!(front.keyboard_focus_on(0));
}

fn menu(parent: usize, anchor_rect: (i32, i32, i32, i32), size: (i32, i32)) -> PopupRequest {
    PopupRequest {
        parent,
        anchor_rect,
        size,
        flip_x: false,
        grab: false,
        color: WHITE,
    }
}

#[test]
fn a_popup_is_placed_at_its_anchor_relative_to_the_window() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = app(&mut server, RED);

    let popup = editor.open_popup(&mut server, &menu(0, (10, 10, 1, 1), (30, 20)));
    roundtrip(&mut server, &mut [&mut editor]);

    assert_eq!(editor.popup_geometry(popup), Some((11, 11, 30, 20)));
    let (x, y) = (CENTERED.0 + 11, CENTERED.1 + 11);
    assert_eq!(pixel(&mut renderer, &mut server, x + 1, y + 1), rgb(WHITE));
    assert_eq!(pixel(&mut renderer, &mut server, x - 1, y - 1), rgb(RED));
    assert_eq!(server.popup_count(), 1);
}

#[test]
fn a_popup_that_would_leave_the_screen_flips_to_the_other_side_of_its_anchor() {
    let mut server = server();
    let mut editor = app(&mut server, RED);

    let request = PopupRequest {
        flip_x: true,
        ..menu(0, (60, 10, 1, 1), (120, 20))
    };
    let popup = editor.open_popup(&mut server, &request);
    roundtrip(&mut server, &mut [&mut editor]);

    assert_eq!(editor.popup_geometry(popup), Some((-60, 11, 120, 20)));
}

#[test]
fn a_click_outside_a_grabbing_popup_dismisses_it_and_is_not_delivered() {
    let mut server = server();
    let mut bystander = app(&mut server, BLUE);
    bystander.set_maximized(&mut server, 0, true);
    bystander.fill_configured_size(&mut server, 0, BLUE);
    let mut editor = app(&mut server, RED);
    click(&mut server, 110, 80);
    roundtrip(&mut server, &mut [&mut bystander, &mut editor]);
    assert!(editor.keyboard_focus_on(0));

    server.press_button(BTN_LEFT, true, 1);
    roundtrip(&mut server, &mut [&mut bystander, &mut editor]);
    let request = PopupRequest {
        grab: true,
        ..menu(0, (10, 10, 1, 1), (30, 20))
    };
    let popup = editor.open_popup(&mut server, &request);
    server.press_button(BTN_LEFT, false, 2);
    roundtrip(&mut server, &mut [&mut bystander, &mut editor]);
    let delivered = bystander.buttons().len();

    click(&mut server, 5, 5);
    roundtrip(&mut server, &mut [&mut bystander, &mut editor]);

    assert!(editor.popup_dismissed(popup));
    assert_eq!(bystander.buttons().len(), delivered);
    assert!(editor.keyboard_focus_on(0));
    assert_eq!(server.popup_count(), 0);
}

const KEY_LEFTCTRL: u32 = 29;
const KEY_F: u32 = 33;
const KEY_M: u32 = 50;
const KEY_LEFTMETA: u32 = 125;

fn chord(server: &mut Headless, keys: &[u32]) {
    for &key in keys {
        server.key(key, true, 0);
    }
    for &key in keys.iter().rev() {
        server.key(key, false, 0);
    }
}

#[test]
fn ctrl_super_f_toggles_full_screen_for_the_key_window() {
    let mut server = server();
    let mut editor = app(&mut server, RED);
    click(&mut server, 110, 80);
    roundtrip(&mut server, &mut [&mut editor]);

    chord(&mut server, &[KEY_LEFTCTRL, KEY_LEFTMETA, KEY_F]);
    roundtrip(&mut server, &mut [&mut editor]);
    let configured = editor.configured(0).expect("a configure");
    assert!(configured.states.contains(&ToplevelState::Fullscreen));
    assert_eq!((configured.width, configured.height), OUTPUT);
    assert!(!editor.keys().iter().any(|(key, _)| *key == KEY_F));

    chord(&mut server, &[KEY_LEFTCTRL, KEY_LEFTMETA, KEY_F]);
    roundtrip(&mut server, &mut [&mut editor]);
    let configured = editor.configured(0).expect("a configure");
    assert!(!configured.states.contains(&ToplevelState::Fullscreen));
}

#[test]
fn super_f_without_ctrl_reaches_the_app() {
    let mut server = server();
    let mut editor = app(&mut server, RED);
    click(&mut server, 110, 80);
    roundtrip(&mut server, &mut [&mut editor]);

    chord(&mut server, &[KEY_LEFTMETA, KEY_F]);
    roundtrip(&mut server, &mut [&mut editor]);

    let configured = editor.configured(0).expect("a configure");
    assert!(!configured.states.contains(&ToplevelState::Fullscreen));
    assert!(editor.keys().contains(&(KEY_F, true)));
}

#[test]
fn super_m_minimizes_the_key_window() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = app(&mut server, RED);
    click(&mut server, 110, 80);
    roundtrip(&mut server, &mut [&mut editor]);

    chord(&mut server, &[KEY_LEFTMETA, KEY_M]);
    roundtrip(&mut server, &mut [&mut editor]);

    assert!(server.windows()[0].is_minimized());
    assert_ne!(pixel(&mut renderer, &mut server, 110, 80), rgb(RED));
    assert!(!editor.keyboard_focus_on(0));
}
