#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use smithay::utils::Size;
use unbound_compositor::{Headless, Renderer};
use unbound_test_wayland::{BLUE, GREEN, RED, ShmImage, TestClient, roundtrip};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (256, 192);
const WINDOW: i32 = 64;
const KEY_A: u32 = 30;

fn solid(color: [u8; 4], width: i32, height: i32) -> Vec<u8> {
    color.repeat((width * height) as usize)
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

fn rgb(bgra: [u8; 4]) -> [u8; 3] {
    [bgra[2], bgra[1], bgra[0]]
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

struct Session {
    server: Headless,
    app: TestClient,
    input_method: TestClient,
}

fn typing_in_a_field_at(cursor: (i32, i32, i32, i32)) -> Session {
    let mut server = Headless::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut input_method = TestClient::connect(&mut server);
    input_method.use_seat(&mut server);
    input_method.become_input_method(&mut server);
    let mut app = TestClient::connect(&mut server);
    app.use_seat(&mut server);
    let pixels = solid(BLUE, WINDOW, WINDOW);
    app.show_window(&mut server, &image(&pixels, WINDOW, WINDOW));
    app.enable_text_input(&mut server, cursor);
    roundtrip(&mut server, &mut [&mut app, &mut input_method]);
    Session {
        server,
        app,
        input_method,
    }
}

#[test]
fn text_committed_by_the_input_method_reaches_the_focused_field() {
    let Session {
        mut server,
        mut app,
        mut input_method,
    } = typing_in_a_field_at((10, 20, 2, 16));
    assert!(
        input_method.input_method_active(),
        "an enabled text field activates the input method"
    );

    input_method.commit_text(&mut server, "日本語");
    roundtrip(&mut server, &mut [&mut app, &mut input_method]);

    assert_eq!(app.committed_text(), ["日本語"]);
}

#[test]
fn keys_go_to_the_input_method_while_it_grabs_the_keyboard() {
    let Session {
        mut server,
        mut app,
        mut input_method,
    } = typing_in_a_field_at((10, 20, 2, 16));
    input_method.grab_keyboard(&mut server);

    server.key(KEY_A, true, 10);
    server.key(KEY_A, false, 11);
    roundtrip(&mut server, &mut [&mut app, &mut input_method]);

    assert_eq!(input_method.grabbed_keys(), [(KEY_A, true), (KEY_A, false)]);
    assert!(
        app.keys().is_empty(),
        "the app sees only what the input method forwards"
    );
}

#[test]
fn candidates_open_below_the_text_cursor() {
    let cursor = (10, 20, 2, 16);
    let Session {
        mut server,
        mut app,
        mut input_method,
    } = typing_in_a_field_at(cursor);
    let candidates = solid(RED, 40, 20);
    input_method.show_candidates(&mut server, &image(&candidates, 40, 20));
    roundtrip(&mut server, &mut [&mut app, &mut input_method]);

    let window = server.windows()[0].location();
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let below = (window.x + cursor.0 + 5, window.y + cursor.1 + cursor.3 + 5);
    assert_eq!(
        pixel(&mut renderer, &mut server, below.0, below.1),
        rgb(RED)
    );
    let cursor_line = (window.x + cursor.0 + 5, window.y + cursor.1 + 5);
    assert_eq!(
        pixel(&mut renderer, &mut server, cursor_line.0, cursor_line.1),
        rgb(BLUE),
        "the candidates leave the text being composed visible"
    );
}

#[test]
fn candidates_open_above_the_cursor_when_the_screen_ends_below_it() {
    let cursor = (10, WINDOW - 8, 2, 4);
    let Session {
        mut server,
        mut app,
        mut input_method,
    } = typing_in_a_field_at(cursor);
    let height = 80;
    let candidates = solid(GREEN, 40, height);
    input_method.show_candidates(&mut server, &image(&candidates, 40, height));
    roundtrip(&mut server, &mut [&mut app, &mut input_method]);

    let window = server.windows()[0].location();
    assert!(
        window.y + cursor.1 + cursor.3 + height > OUTPUT.1,
        "the test window sits too high to need the flip"
    );
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let above = (window.x + cursor.0 + 5, window.y + cursor.1 - 5);
    assert_eq!(
        pixel(&mut renderer, &mut server, above.0, above.1),
        rgb(GREEN)
    );
    let below = (window.x + cursor.0 + 5, window.y + cursor.1 + cursor.3 + 2);
    assert_eq!(
        pixel(&mut renderer, &mut server, below.0, below.1),
        rgb(BLUE),
        "nothing is drawn below the cursor"
    );
}
