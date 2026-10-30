use crate::support::*;
use smithay::utils::{Point, Size};
use unbound_compositor::{ARROW_FILL, ARROW_OUTLINE, BACKGROUND, Headless, Renderer};
use unbound_test_wayland::{RED, TestClient, roundtrip};

const KEY_W: u32 = 17;

fn client_with_window(server: &mut Headless, side: i32) -> TestClient {
    window(server, RED, (side, side))
}

fn click(server: &mut Headless, at: (f64, f64)) {
    server.move_pointer_to(Point::from(at), 1);
    server.press_button(BTN_LEFT, true, 2);
    server.press_button(BTN_LEFT, false, 3);
}

fn tap(server: &mut Headless, key: u32) {
    server.key(key, true, 10);
    server.key(key, false, 11);
}

fn chord(server: &mut Headless, key: u32) {
    server.key(KEY_LEFTMETA, true, 20);
    tap(server, key);
    server.key(KEY_LEFTMETA, false, 21);
}

#[test]
fn clicking_a_window_behind_focuses_and_raises_it() {
    let mut server = server();
    let mut big = client_with_window(&mut server, 128);
    let mut small = client_with_window(&mut server, 32);
    roundtrip(&mut server, &mut [&mut big, &mut small]);
    assert!(small.keyboard_focus_on(0));

    let corner = server.windows()[0].location();
    click(
        &mut server,
        (f64::from(corner.x) + 4.0, f64::from(corner.y) + 4.0),
    );
    roundtrip(&mut server, &mut [&mut big, &mut small]);

    assert!(big.keyboard_focus_on(0));
    assert!(!small.keyboard_focus_on(0));
    assert_eq!(
        server.windows().last().map(|window| window.size()),
        Some(Some(Size::from((128, 128))))
    );
}

#[test]
fn the_pointer_enters_the_surface_under_it_at_surface_coordinates() {
    let mut server = server();
    let mut client = client_with_window(&mut server, 64);
    let origin = server.windows()[0].location();

    server.move_pointer_to(
        Point::from((f64::from(origin.x) + 10.0, f64::from(origin.y) + 20.0)),
        1,
    );
    client.exchange(&mut server);

    assert_eq!(client.pointer_focus(), Some((0, 10.0, 20.0)));
}

#[test]
fn a_window_that_appears_under_a_still_pointer_receives_the_pointer() {
    let mut server = server();
    let still = (128.0, 96.0);
    server.move_pointer_to(Point::from(still), 1);

    let mut client = client_with_window(&mut server, 64);
    client.exchange(&mut server);

    let origin = server.windows()[0].location();
    assert_eq!(
        client.pointer_focus(),
        Some((
            0,
            still.0 - f64::from(origin.x),
            still.1 - f64::from(origin.y)
        ))
    );
}

#[test]
fn a_held_button_keeps_the_pointer_on_its_surface_without_extra_motion() {
    let mut server = server();
    let mut pressed = client_with_window(&mut server, 64);
    let origin = server.windows()[0].location();
    server.move_pointer_to(
        Point::from((f64::from(origin.x) + 10.0, f64::from(origin.y) + 10.0)),
        1,
    );
    server.press_button(BTN_LEFT, true, 2);
    pressed.exchange(&mut server);
    let motions = pressed.pointer_motions();

    let mut above = client_with_window(&mut server, 96);
    roundtrip(&mut server, &mut [&mut pressed, &mut above]);
    roundtrip(&mut server, &mut [&mut pressed, &mut above]);

    assert_eq!(pressed.pointer_motions(), motions);
    assert_eq!(pressed.pointer_focus().map(|(index, _, _)| index), Some(0));
    assert_eq!(above.pointer_focus(), None);
}

#[test]
fn keys_reach_only_the_focused_client() {
    let mut server = server();
    let mut first = client_with_window(&mut server, 128);
    let mut second = client_with_window(&mut server, 32);
    roundtrip(&mut server, &mut [&mut first, &mut second]);

    tap(&mut server, KEY_A);
    roundtrip(&mut server, &mut [&mut first, &mut second]);

    assert_eq!(second.keys(), [(KEY_A, true), (KEY_A, false)]);
    assert!(first.keys().is_empty());
}

#[test]
fn clicking_the_empty_desktop_leaves_no_window_focused() {
    let mut server = server();
    let mut client = client_with_window(&mut server, 32);
    assert!(client.keyboard_focus_on(0));

    click(&mut server, (1.0, 1.0));
    client.exchange(&mut server);

    assert!(!client.keyboard_focus_on(0));
}

#[test]
fn super_w_asks_the_focused_window_to_close_instead_of_typing() {
    let mut server = server();
    let mut client = client_with_window(&mut server, 32);

    chord(&mut server, KEY_W);
    client.exchange(&mut server);

    assert!(client.close_requested(0));
    assert!(!client.keys().iter().any(|(key, _)| *key == KEY_W));
}

#[test]
fn super_q_asks_every_window_of_the_focused_app_to_close() {
    let mut server = server();
    let mut other = client_with_window(&mut server, 128);
    let mut app = client_with_window(&mut server, 32);
    let pixels = solid(RED, (16, 16));
    app.show_window(&mut server, &xrgb(&pixels, (16, 16)));
    roundtrip(&mut server, &mut [&mut other, &mut app]);

    chord(&mut server, KEY_Q);
    roundtrip(&mut server, &mut [&mut other, &mut app]);

    assert!(app.close_requested(0));
    assert!(app.close_requested(1));
    assert!(!other.close_requested(0));
}

#[test]
fn super_q_quits_every_process_of_the_focused_app() {
    let mut server = server();
    let pixels = solid(RED, (32, 32));
    let mut other = TestClient::connect(&mut server);
    other.use_seat(&mut server);
    other.show_window_of(&mut server, &xrgb(&pixels, (32, 32)), "org.example.Editor");
    let mut first = TestClient::connect(&mut server);
    first.use_seat(&mut server);
    first.show_window_of(
        &mut server,
        &xrgb(&pixels, (32, 32)),
        "org.example.Terminal",
    );
    let mut second = TestClient::connect(&mut server);
    second.use_seat(&mut server);
    second.show_window_of(
        &mut server,
        &xrgb(&pixels, (32, 32)),
        "org.example.Terminal",
    );
    roundtrip(&mut server, &mut [&mut other, &mut first, &mut second]);

    chord(&mut server, KEY_Q);
    roundtrip(&mut server, &mut [&mut other, &mut first, &mut second]);

    assert!(first.close_requested(0));
    assert!(second.close_requested(0));
    assert!(!other.close_requested(0));
}

#[test]
fn super_q_quits_the_active_app_after_it_closed_its_last_window() {
    let mut server = server();
    let mut other = client_with_window(&mut server, 128);
    other.take_app_requests(&mut server);
    let mut app = client_with_window(&mut server, 32);
    app.take_app_requests(&mut server);
    roundtrip(&mut server, &mut [&mut other, &mut app]);
    app.close_window(&mut server, 0);
    roundtrip(&mut server, &mut [&mut other, &mut app]);

    chord(&mut server, KEY_Q);
    roundtrip(&mut server, &mut [&mut other, &mut app]);

    assert_eq!(app.quit_requests(), 1);
    assert_eq!(other.quit_requests(), 0);
}

#[test]
fn super_q_asks_an_app_that_takes_app_requests_to_quit_once_and_closes_nothing() {
    let mut server = server();
    let mut other = client_with_window(&mut server, 128);
    other.take_app_requests(&mut server);
    let mut app = client_with_window(&mut server, 32);
    app.take_app_requests(&mut server);
    let pixels = solid(RED, (16, 16));
    app.show_window(&mut server, &xrgb(&pixels, (16, 16)));
    roundtrip(&mut server, &mut [&mut other, &mut app]);

    chord(&mut server, KEY_Q);
    roundtrip(&mut server, &mut [&mut other, &mut app]);

    assert_eq!(app.quit_requests(), 1);
    assert!(!app.close_requested(0));
    assert!(!app.close_requested(1));
    assert_eq!(other.quit_requests(), 0);
}

#[test]
fn super_option_escape_opens_force_quit_without_typing_escape() {
    let mut server = server();
    let mut client = client_with_window(&mut server, 32);
    server.set_force_quit(std::path::Path::new("/bin/true"));

    server.key(KEY_LEFTALT, true, 19);
    chord(&mut server, KEY_ESC);
    server.key(KEY_LEFTALT, false, 22);
    client.exchange(&mut server);
    let upkeep = server.keep_session_clients_running();

    assert!(upkeep.failures.is_empty());
    assert!(server.force_quit_pid().is_some());
    assert!(!client.keys().iter().any(|(key, _)| *key == KEY_ESC));
}

#[test]
fn the_compositor_draws_an_arrow_where_the_pointer_is_once_it_moves() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    renderer.render(&mut server).expect("first frame");
    let rgba = |snapshot: &unbound_compositor::Snapshot, x: i32, y: i32| {
        let at = ((y * snapshot.size.w + x) * 4) as usize;
        [
            snapshot.rgba[at],
            snapshot.rgba[at + 1],
            snapshot.rgba[at + 2],
        ]
    };
    let before = renderer.snapshot(&server).expect("read back");
    assert_eq!(rgba(&before, 40, 50), BACKGROUND);

    server.move_pointer_to(Point::from((40.0, 50.0)), 1);
    renderer
        .render(&mut server)
        .expect("frame with the pointer");
    let after = renderer.snapshot(&server).expect("read back");

    let bgr = |bgra: [u8; 4]| [bgra[2], bgra[1], bgra[0]];
    assert_eq!(rgba(&after, 40, 50), bgr(ARROW_OUTLINE));
    assert_eq!(rgba(&after, 41, 55), bgr(ARROW_FILL));
}
