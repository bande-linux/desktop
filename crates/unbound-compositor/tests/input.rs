use smithay::utils::{Point, Size};
use unbound_compositor::{ARROW_FILL, ARROW_OUTLINE, BACKGROUND, Headless, Renderer};
use unbound_test_wayland::{RED, ShmImage, TestClient, roundtrip};
use wayland_client::protocol::wl_shm::Format;

const KEY_TAB: u32 = 15;
const KEY_Q: u32 = 16;
const KEY_W: u32 = 17;
const KEY_A: u32 = 30;
const KEY_LEFTMETA: u32 = 125;
const BTN_LEFT: u32 = 0x110;

fn server() -> Headless {
    Headless::new(Size::from((256, 192))).expect("headless compositor")
}

fn solid(side: usize) -> Vec<u8> {
    RED.repeat(side * side)
}

fn image(pixels: &[u8], side: i32) -> ShmImage<'_> {
    ShmImage {
        width: side,
        height: side,
        stride: side * 4,
        format: Format::Xrgb8888,
        bytes: pixels,
    }
}

fn client_with_window(server: &mut Headless, side: i32) -> TestClient {
    let mut client = TestClient::connect(server);
    client.use_seat(server);
    let pixels = solid(side as usize);
    client.show_window(server, &image(&pixels, side));
    client
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
    let pixels = solid(16);
    app.show_window(&mut server, &image(&pixels, 16));
    roundtrip(&mut server, &mut [&mut other, &mut app]);

    chord(&mut server, KEY_Q);
    roundtrip(&mut server, &mut [&mut other, &mut app]);

    assert!(app.close_requested(0));
    assert!(app.close_requested(1));
    assert!(!other.close_requested(0));
}

#[test]
fn super_tab_activates_the_most_recent_other_app() {
    let mut server = server();
    let mut first = client_with_window(&mut server, 128);
    let mut second = client_with_window(&mut server, 32);
    roundtrip(&mut server, &mut [&mut first, &mut second]);
    assert!(second.keyboard_focus_on(0));

    chord(&mut server, KEY_TAB);
    roundtrip(&mut server, &mut [&mut first, &mut second]);

    assert!(first.keyboard_focus_on(0));
    assert!(second.keys().iter().all(|(key, _)| *key != KEY_TAB));
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
