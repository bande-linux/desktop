#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use smithay::utils::{Logical, Point, Size};
use unbound_compositor::{Headless, Rect, Renderer};
use unbound_test_wayland::{DecorationMode, RED, ShmImage, TestClient, ToplevelState, roundtrip};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (256, 192);
const WINDOW: (i32, i32) = (120, 40);
const BAR: i32 = 40;
const CLOSE: (i32, i32) = (24, BAR / 2);
const BTN_LEFT: u32 = 0x110;

fn decorated(server: &mut Headless, requested: Option<DecorationMode>) -> TestClient {
    decorated_sized(server, requested, WINDOW)
}

fn decorated_sized(
    server: &mut Headless,
    requested: Option<DecorationMode>,
    size: (i32, i32),
) -> TestClient {
    let mut client = TestClient::connect(server);
    client.use_seat(server);
    let pixels = RED.repeat((size.0 * size.1) as usize);
    let image = ShmImage {
        width: size.0,
        height: size.1,
        stride: size.0 * 4,
        format: Format::Xrgb8888,
        bytes: &pixels,
    };
    client.show_decorated_window(server, &image, requested);
    client.exchange(server);
    client.commit_buffer(0, &image, (0, 0, size.0, size.1));
    client.exchange(server);
    assert_eq!(client.protocol_error(), None);
    client
}

fn server() -> Headless {
    let mut server = Headless::new(Size::from(OUTPUT)).expect("headless compositor");
    server.set_reduce_motion(true);
    server
}

fn geometry(server: &Headless) -> Rect {
    server.windows()[0].geometry().expect("a mapped window")
}

fn origin(server: &Headless) -> (i32, i32) {
    let location = geometry(server).location;
    (location.x, location.y)
}

fn at(x: i32, y: i32) -> Point<f64, Logical> {
    (f64::from(x), f64::from(y)).into()
}

fn pixel(server: &mut Headless, x: i32, y: i32) -> [u8; 3] {
    let mut renderer = Renderer::new().expect("renderer");
    renderer.render(server).expect("frame");
    let snapshot = renderer.snapshot(server).expect("read back");
    let at = ((y * snapshot.size.w + x) * 4) as usize;
    [
        snapshot.rgba[at],
        snapshot.rgba[at + 1],
        snapshot.rgba[at + 2],
    ]
}

fn close_to(actual: [u8; 3], expected: [u8; 3]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(actual, expected)| actual.abs_diff(expected) <= 6)
}

fn click(server: &mut Headless, client: &mut TestClient, x: i32, y: i32, time: u32) {
    server.move_pointer_to(at(x, y), time);
    server.press_button(BTN_LEFT, true, time);
    server.press_button(BTN_LEFT, false, time + 1);
    roundtrip(server, &mut [client]);
}

#[test]
fn a_window_that_asks_for_server_side_decorations_gets_our_title_bar_above_its_content() {
    let mut server = server();
    let client = decorated(&mut server, Some(DecorationMode::ServerSide));
    let top_left = origin(&server);

    assert_eq!(
        client.decoration_modes().last(),
        Some(&DecorationMode::ServerSide)
    );
    assert_eq!(geometry(&server).size, (WINDOW.0, WINDOW.1 + BAR).into());
    assert!(close_to(
        pixel(&mut server, top_left.0 + 100, top_left.1 + 3),
        [0xEC, 0xEA, 0xE7]
    ));
    assert!(close_to(
        pixel(&mut server, top_left.0 + 14, top_left.1 + BAR / 2),
        [0xE2, 0xDF, 0xDB]
    ));
    assert!(close_to(
        pixel(&mut server, top_left.0 + 60, top_left.1 + BAR + 10),
        [0xFF, 0x00, 0x00]
    ));
}

#[test]
fn a_window_that_leaves_the_choice_to_the_compositor_gets_our_title_bar() {
    let mut server = server();
    let client = decorated(&mut server, None);

    assert_eq!(
        client.decoration_modes().last(),
        Some(&DecorationMode::ServerSide)
    );
    assert_eq!(geometry(&server).size, (WINDOW.0, WINDOW.1 + BAR).into());
}

#[test]
fn a_window_that_draws_its_own_decorations_gets_no_title_bar() {
    let mut server = server();
    let client = decorated(&mut server, Some(DecorationMode::ClientSide));

    assert_eq!(
        client.decoration_modes().last(),
        Some(&DecorationMode::ClientSide)
    );
    assert_eq!(geometry(&server).size, WINDOW.into());
}

#[test]
fn the_title_bar_follows_dark_mode() {
    let mut server = server();
    let _client = decorated(&mut server, Some(DecorationMode::ServerSide));
    let top_left = origin(&server);

    server.set_dark_appearance(true);

    assert!(close_to(
        pixel(&mut server, top_left.0 + 100, top_left.1 + 3),
        [0x14, 0x12, 0x10]
    ));
}

#[test]
fn only_the_close_button_turns_red_under_the_pointer() {
    let mut server = server();
    let _client = decorated(&mut server, Some(DecorationMode::ServerSide));
    let top_left = origin(&server);
    let beside_the_cross = (top_left.0 + CLOSE.0 + 5, top_left.1 + CLOSE.1);
    let well = [0xE2, 0xDF, 0xDB];

    assert!(close_to(
        pixel(&mut server, beside_the_cross.0, beside_the_cross.1),
        well
    ));
    server.move_pointer_to(at(top_left.0 + CLOSE.0 + 21, top_left.1 + CLOSE.1), 10);
    assert!(close_to(
        pixel(&mut server, beside_the_cross.0, beside_the_cross.1),
        well
    ));
    server.move_pointer_to(at(top_left.0 + CLOSE.0, top_left.1 + CLOSE.1), 11);
    assert!(close_to(
        pixel(&mut server, beside_the_cross.0, beside_the_cross.1),
        [0xD4, 0x2F, 0x34]
    ));
}

#[test]
fn the_close_button_asks_the_app_to_close_its_window() {
    let mut server = server();
    let mut client = decorated(&mut server, Some(DecorationMode::ServerSide));
    let top_left = origin(&server);

    click(
        &mut server,
        &mut client,
        top_left.0 + CLOSE.0,
        top_left.1 + CLOSE.1,
        10,
    );

    assert!(client.close_requested(0));
}

#[test]
fn a_press_that_slides_off_the_close_button_closes_nothing() {
    let mut server = server();
    let mut client = decorated(&mut server, Some(DecorationMode::ServerSide));
    let top_left = origin(&server);

    server.move_pointer_to(at(top_left.0 + CLOSE.0, top_left.1 + CLOSE.1), 10);
    server.press_button(BTN_LEFT, true, 10);
    server.move_pointer_to(at(top_left.0 + 90, top_left.1 + BAR / 2), 11);
    server.press_button(BTN_LEFT, false, 12);
    roundtrip(&mut server, &mut [&mut client]);

    assert!(!client.close_requested(0));
}

#[test]
fn dragging_the_title_bar_moves_the_window() {
    let mut server = server();
    let mut client = decorated(&mut server, Some(DecorationMode::ServerSide));
    let top_left = origin(&server);

    server.move_pointer_to(at(top_left.0 + 90, top_left.1 + 10), 10);
    server.press_button(BTN_LEFT, true, 10);
    server.move_pointer_to(at(top_left.0 + 70, top_left.1 + 40), 11);
    server.press_button(BTN_LEFT, false, 12);
    roundtrip(&mut server, &mut [&mut client]);

    assert_eq!(
        geometry(&server).location,
        (top_left.0 - 20, top_left.1 + 30).into()
    );
}

#[test]
fn dragging_the_edge_resizes_the_app_without_its_title_bar() {
    let mut server = server();
    let mut client = decorated(&mut server, Some(DecorationMode::ServerSide));
    let top_left = origin(&server);

    let corner = (top_left.0 + WINDOW.0 + 2, top_left.1 + WINDOW.1 + BAR + 2);
    server.move_pointer_to(at(corner.0, corner.1), 10);
    server.press_button(BTN_LEFT, true, 10);
    server.move_pointer_to(at(corner.0 + 30, corner.1 + 20), 11);
    roundtrip(&mut server, &mut [&mut client]);

    let configured = client.configured(0).expect("a configure");
    assert_eq!(
        (configured.width, configured.height),
        (WINDOW.0 + 30, WINDOW.1 + 20)
    );
}

#[test]
fn double_clicking_the_title_bar_zooms_the_window_to_fill_the_screen_with_its_title_bar() {
    let mut server = server();
    let mut client = decorated(&mut server, Some(DecorationMode::ServerSide));
    let top_left = origin(&server);

    click(
        &mut server,
        &mut client,
        top_left.0 + 90,
        top_left.1 + 10,
        10,
    );
    click(
        &mut server,
        &mut client,
        top_left.0 + 90,
        top_left.1 + 10,
        100,
    );

    let configured = client.configured(0).expect("a configure");
    assert!(configured.states.contains(&ToplevelState::Maximized));
    assert_eq!(
        (configured.width, configured.height),
        (OUTPUT.0, OUTPUT.1 - BAR)
    );
}

#[test]
fn a_window_taller_than_the_screen_opens_with_its_title_bar_on_screen() {
    let mut server = server();
    let _client = decorated_sized(
        &mut server,
        Some(DecorationMode::ServerSide),
        (OUTPUT.0 + 40, OUTPUT.1 + 40),
    );

    assert_eq!(geometry(&server).location, (0, 0).into());
}
