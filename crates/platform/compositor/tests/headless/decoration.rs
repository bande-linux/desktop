use crate::support::*;
use smithay::utils::Size;
use unbound_compositor::{Compositor, Renderer};
use unbound_test_wayland::{DecorationMode, RED, TestClient, ToplevelState, roundtrip};

const WINDOW: (i32, i32) = (120, 40);
const BAR: i32 = 40;
const CLOSE: (i32, i32) = (24, BAR / 2);
const ZOOM: (i32, i32) = (CLOSE.0 + 42, BAR / 2);

fn decorated(server: &mut Compositor, requested: Option<DecorationMode>) -> TestClient {
    decorated_sized(server, requested, WINDOW)
}

fn decorated_sized(
    server: &mut Compositor,
    requested: Option<DecorationMode>,
    size: (i32, i32),
) -> TestClient {
    let mut client = TestClient::connect(server);
    client.use_seat(server);
    let pixels = solid(RED, size);
    let image = xrgb(&pixels, size);
    client.show_decorated_window(server, &image, requested);
    client.exchange(server);
    client.commit_buffer(0, &image, (0, 0, size.0, size.1));
    client.exchange(server);
    assert_eq!(client.protocol_error(), None);
    client
}

fn server() -> Compositor {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    server.set_reduce_motion(true);
    server
}

fn origin(server: &Compositor) -> (i32, i32) {
    let location = geometry(server).location;
    (location.x, location.y)
}

fn close_to(actual: [u8; 3], expected: [u8; 3]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(actual, expected)| actual.abs_diff(expected) <= 6)
}

fn click(server: &mut Compositor, client: &mut TestClient, x: i32, y: i32, time: u32) {
    server.move_pointer_to(at(x, y), time);
    server.press_button(BTN_LEFT, true, time);
    server.press_button(BTN_LEFT, false, time + 1);
    roundtrip(server, &mut [client]);
}

#[test]
fn a_window_that_asks_for_server_side_decorations_gets_our_title_bar_above_its_content() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let client = decorated(&mut server, Some(DecorationMode::ServerSide));
    let top_left = origin(&server);

    assert_eq!(
        client.decoration_modes().last(),
        Some(&DecorationMode::ServerSide)
    );
    assert_eq!(geometry(&server).size, (WINDOW.0, WINDOW.1 + BAR).into());
    assert!(close_to(
        pixel(&mut renderer, &mut server, top_left.0 + 100, top_left.1 + 3),
        [0xEC, 0xEA, 0xE7]
    ));
    assert!(close_to(
        pixel(
            &mut renderer,
            &mut server,
            top_left.0 + 14,
            top_left.1 + BAR / 2
        ),
        [0xE2, 0xDF, 0xDB]
    ));
    assert!(close_to(
        pixel(
            &mut renderer,
            &mut server,
            top_left.0 + 60,
            top_left.1 + BAR + 10
        ),
        [0xFF, 0x00, 0x00]
    ));
}

#[test]
fn a_window_gets_our_title_bar_unless_it_draws_its_own_decorations() {
    for (requested, decided, height) in [
        (None, DecorationMode::ServerSide, WINDOW.1 + BAR),
        (
            Some(DecorationMode::ClientSide),
            DecorationMode::ClientSide,
            WINDOW.1,
        ),
    ] {
        let mut server = server();
        let client = decorated(&mut server, requested);

        assert_eq!(client.decoration_modes().last(), Some(&decided));
        assert_eq!(geometry(&server).size, (WINDOW.0, height).into());
    }
}

#[test]
fn the_title_bar_follows_dark_mode() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let _client = decorated(&mut server, Some(DecorationMode::ServerSide));
    let top_left = origin(&server);

    server.set_dark_appearance(true);

    assert!(close_to(
        pixel(&mut renderer, &mut server, top_left.0 + 100, top_left.1 + 3),
        [0x14, 0x12, 0x10]
    ));
}

#[test]
fn only_the_close_button_turns_red_under_the_pointer() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let _client = decorated(&mut server, Some(DecorationMode::ServerSide));
    let top_left = origin(&server);
    let beside_the_cross = (top_left.0 + CLOSE.0 + 5, top_left.1 + CLOSE.1);
    let well = [0xE2, 0xDF, 0xDB];

    assert!(close_to(
        pixel(
            &mut renderer,
            &mut server,
            beside_the_cross.0,
            beside_the_cross.1
        ),
        well
    ));
    server.move_pointer_to(at(top_left.0 + CLOSE.0 + 21, top_left.1 + CLOSE.1), 10);
    assert!(close_to(
        pixel(
            &mut renderer,
            &mut server,
            beside_the_cross.0,
            beside_the_cross.1
        ),
        well
    ));
    server.move_pointer_to(at(top_left.0 + CLOSE.0, top_left.1 + CLOSE.1), 11);
    assert!(close_to(
        pixel(
            &mut renderer,
            &mut server,
            beside_the_cross.0,
            beside_the_cross.1
        ),
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

#[test]
fn holding_alt_redraws_the_zoom_button_as_a_plus() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let _client = decorated(&mut server, Some(DecorationMode::ServerSide));
    let top_left = origin(&server);
    let centre = (top_left.0 + ZOOM.0, top_left.1 + ZOOM.1);
    let well = [0xE2, 0xDF, 0xDB];
    let output = server.main_output();

    assert!(close_to(
        pixel(&mut renderer, &mut server, centre.0, centre.1),
        well
    ));
    while server.take_redraw_on(&output) {}
    server.key(KEY_LEFTALT, true, 10);
    assert!(
        server.take_redraw_on(&output),
        "holding Alt asks for a new frame"
    );
    assert!(!close_to(
        pixel(&mut renderer, &mut server, centre.0, centre.1),
        well
    ));
    server.key(KEY_LEFTALT, false, 11);
    assert!(server.take_redraw_on(&output));
    assert!(close_to(
        pixel(&mut renderer, &mut server, centre.0, centre.1),
        well
    ));
}
