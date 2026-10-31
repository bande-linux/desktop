use std::time::Duration;

use bande_compositor::{Compositor, Renderer, Snapshot};
use bande_test_wayland::{
    Anchor, GREEN, KeyboardInteractivity, Layer, LayerRequest, RED, ShmImage, TestClient, roundtrip,
};
use smithay::utils::{Logical, Point, Rectangle, Size};
use wayland_client::protocol::wl_shm::Format;

pub const OUTPUT: (i32, i32) = (256, 192);
pub const BTN_LEFT: u32 = 0x110;
pub const KEY_ESC: u32 = 1;
pub const KEY_TAB: u32 = 15;
pub const KEY_Q: u32 = 16;
pub const KEY_LEFTCTRL: u32 = 29;
pub const KEY_A: u32 = 30;
pub const KEY_F: u32 = 33;
pub const KEY_H: u32 = 35;
pub const KEY_LEFTSHIFT: u32 = 42;
pub const KEY_M: u32 = 50;
pub const KEY_LEFTALT: u32 = 56;
pub const KEY_SPACE: u32 = 57;
pub const KEY_UP: u32 = 103;
pub const KEY_LEFT: u32 = 105;
pub const KEY_RIGHT: u32 = 106;
pub const KEY_DOWN: u32 = 108;
pub const KEY_LEFTMETA: u32 = 125;
pub const EDITOR: &str = "org.example.Editor";
pub const VIEWER: &str = "org.example.Viewer";

pub fn server() -> Compositor {
    Compositor::new(Size::from(OUTPUT)).expect("headless compositor")
}

pub fn solid(color: [u8; 4], (width, height): (i32, i32)) -> Vec<u8> {
    color.repeat((width * height) as usize)
}

pub fn xrgb(bytes: &[u8], (width, height): (i32, i32)) -> ShmImage<'_> {
    ShmImage {
        width,
        height,
        stride: width * 4,
        format: Format::Xrgb8888,
        bytes,
    }
}

pub fn window(server: &mut Compositor, color: [u8; 4], size: (i32, i32)) -> TestClient {
    let mut client = TestClient::connect(server);
    client.use_seat(server);
    client.show_window(server, &xrgb(&solid(color, size), size));
    client
}

pub fn focused_app(server: &mut Compositor) -> TestClient {
    let mut app = window(server, RED, (32, 32));
    roundtrip(server, &mut [&mut app]);
    assert!(app.keyboard_focus_on(0));
    app
}

pub fn window_of(
    server: &mut Compositor,
    color: [u8; 4],
    size: (i32, i32),
    app_id: &str,
) -> TestClient {
    let mut client = window(server, color, size);
    client.set_app_id(server, 0, app_id);
    client
}

pub fn at(x: i32, y: i32) -> Point<f64, Logical> {
    (f64::from(x), f64::from(y)).into()
}

pub fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
    Rectangle::new((x, y).into(), (w, h).into())
}

pub fn click(server: &mut Compositor, x: i32, y: i32) {
    server.move_pointer_to(at(x, y), 0);
    server.press_button(BTN_LEFT, true, 0);
    server.press_button(BTN_LEFT, false, 0);
}

pub fn tap(server: &mut Compositor, key: u32) {
    server.key(key, true, 0);
    server.key(key, false, 0);
}

pub fn chord(server: &mut Compositor, keys: &[u32]) {
    for &key in keys {
        server.key(key, true, 0);
    }
    for &key in keys.iter().rev() {
        server.key(key, false, 0);
    }
}

pub fn settle(server: &mut Compositor, from: u64) -> u64 {
    let mut at = from;
    while {
        at += 16;
        server.animate(Duration::from_millis(at));
        server.is_animating()
    } {}
    at
}

pub fn rgb(bgra: [u8; 4]) -> [u8; 3] {
    [bgra[2], bgra[1], bgra[0]]
}

pub fn rgb_at(snapshot: &Snapshot, x: i32, y: i32) -> [u8; 3] {
    let at = ((y * snapshot.size.w + x) * 4) as usize;
    [
        snapshot.rgba[at],
        snapshot.rgba[at + 1],
        snapshot.rgba[at + 2],
    ]
}

pub fn pixel(renderer: &mut Renderer, server: &mut Compositor, x: i32, y: i32) -> [u8; 3] {
    renderer.render(server).expect("frame");
    rgb_at(&renderer.snapshot(server).expect("read back"), x, y)
}

pub const BAR: i32 = 24;

pub fn top_bar(server: &mut Compositor) -> TestClient {
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

pub fn left_rail(server: &mut Compositor) -> TestClient {
    let mut shell = TestClient::connect_as_shell(server);
    shell.show_layer(
        server,
        &LayerRequest {
            layer: Layer::Top,
            anchor: Anchor::Left | Anchor::Top | Anchor::Bottom,
            size: (BAR as u32, 0),
            exclusive_zone: BAR,
            keyboard: KeyboardInteractivity::None,
            color: GREEN,
        },
    );
    shell
}

pub fn geometry(server: &Compositor) -> Rectangle<i32, Logical> {
    server.windows()[0].geometry().expect("a mapped window")
}
