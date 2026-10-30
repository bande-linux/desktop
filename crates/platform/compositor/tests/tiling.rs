#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use smithay::utils::{Logical, Point, Size};
use unbound_compositor::{BACKGROUND, Headless, Rect, Renderer};
use unbound_test_wayland::{BLUE, Placement, RED, ResizeEdge, ShmImage, TestClient, roundtrip};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (256, 192);
const WINDOW: (i32, i32) = (64, 48);
const BTN_LEFT: u32 = 0x110;
const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTMETA: u32 = 125;
const KEY_UP: u32 = 103;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_DOWN: u32 = 108;
const GRAB: (i32, i32) = (110, 80);

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

fn at(x: i32, y: i32) -> Point<f64, Logical> {
    (f64::from(x), f64::from(y)).into()
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect {
        location: (x, y).into(),
        size: (w, h).into(),
    }
}

fn grab(server: &mut Headless, client: &mut TestClient) {
    server.move_pointer_to(at(GRAB.0, GRAB.1), 0);
    server.press_button(BTN_LEFT, true, 0);
    roundtrip(server, &mut [client]);
    client.request_move(server, 0);
}

fn release(server: &mut Headless, client: &mut TestClient, time: u32) {
    server.press_button(BTN_LEFT, false, time);
    roundtrip(server, &mut [client]);
}

fn drop_after(server: &mut Headless, client: &mut TestClient, to: (i32, i32), rest: u32) {
    grab(server, client);
    server.move_pointer_to(at(to.0, to.1), 1000);
    server.snap_tick(Duration::from_millis(u64::from(1000 + rest)));
    release(server, client, 1000 + rest);
}

fn settle(server: &mut Headless, client: &mut TestClient, color: [u8; 4]) -> Rect {
    if client
        .configured(0)
        .is_some_and(|configured| configured.width > 0)
    {
        client.fill_configured_size(server, 0, color);
    }
    roundtrip(server, &mut [client]);
    server.windows()[0].geometry().expect("mapped")
}

fn chord(server: &mut Headless, client: &mut TestClient, key: u32) {
    for (code, pressed) in [
        (KEY_LEFTCTRL, true),
        (KEY_LEFTMETA, true),
        (key, true),
        (key, false),
        (KEY_LEFTMETA, false),
        (KEY_LEFTCTRL, false),
    ] {
        server.key(code, pressed, 0);
    }
    roundtrip(server, &mut [client]);
}

#[test]
fn a_window_dropped_on_the_left_edge_after_resting_fills_the_left_half() {
    let mut server = server();
    let mut editor = app(&mut server, RED);

    drop_after(&mut server, &mut editor, (0, 96), 150);

    assert_eq!(settle(&mut server, &mut editor, RED), rect(0, 0, 128, 192));
}

#[test]
fn a_window_swept_past_the_edge_without_resting_is_not_tiled() {
    let mut server = server();
    let mut editor = app(&mut server, RED);

    drop_after(&mut server, &mut editor, (0, 96), 100);

    let geometry = settle(&mut server, &mut editor, RED);
    assert_eq!(geometry.size, WINDOW.into());
}

#[test]
fn corners_give_quarters_and_the_top_edge_zooms() {
    for (to, expected) in [
        ((255, 4), rect(128, 0, 128, 96)),
        ((0, 190), rect(0, 96, 128, 96)),
        ((128, 0), rect(0, 0, 256, 192)),
    ] {
        let mut server = server();
        let mut editor = app(&mut server, RED);

        drop_after(&mut server, &mut editor, to, 200);

        assert_eq!(settle(&mut server, &mut editor, RED), expected, "{to:?}");
    }
}

#[test]
fn the_preview_appears_only_once_the_pointer_rests() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = app(&mut server, RED);
    let mut background = |server: &mut Headless, time: u64| {
        server.animate(Duration::from_millis(time));
        renderer.render(server).expect("frame");
        let snapshot = renderer.snapshot(server).expect("read back");
        let at = ((20 * snapshot.size.w + 60) * 4) as usize;
        [
            snapshot.rgba[at],
            snapshot.rgba[at + 1],
            snapshot.rgba[at + 2],
        ]
    };
    grab(&mut server, &mut editor);
    server.move_pointer_to(at(0, 96), 1000);

    assert_eq!(
        server.snap_tick(Duration::from_millis(1100)),
        Some(Duration::from_millis(1150))
    );
    assert_eq!(background(&mut server, 1100), BACKGROUND);
    assert_eq!(server.snap_tick(Duration::from_millis(1150)), None);
    background(&mut server, 1150);
    let previewed = background(&mut server, 2000);
    assert!(
        previewed
            .iter()
            .zip(BACKGROUND)
            .all(|(shown, below)| *shown > below)
    );

    release(&mut server, &mut editor, 2000);
    editor.fill_configured_size(&mut server, 0, RED);
    background(&mut server, 2100);
    assert_eq!(background(&mut server, 4000), [0xff, 0, 0]);
}

#[test]
fn ctrl_super_arrows_walk_through_halves_quarters_zoom_and_back() {
    let mut server = server();
    let mut editor = app(&mut server, RED);
    let mut steps = Vec::new();
    for key in [KEY_LEFT, KEY_UP, KEY_DOWN, KEY_DOWN, KEY_RIGHT] {
        chord(&mut server, &mut editor, key);
        steps.push(settle(&mut server, &mut editor, RED));
    }

    assert_eq!(
        steps,
        [
            rect(0, 0, 128, 192),
            rect(0, 0, 128, 96),
            rect(0, 0, 128, 192),
            rect(0, 96, 128, 96),
            rect(128, 0, 128, 192),
        ]
    );
}

#[test]
fn ctrl_super_up_zooms_a_free_window_and_down_returns_it() {
    let mut server = server();
    let mut editor = app(&mut server, RED);
    let free = settle(&mut server, &mut editor, RED);

    chord(&mut server, &mut editor, KEY_UP);
    assert_eq!(settle(&mut server, &mut editor, RED), rect(0, 0, 256, 192));
    chord(&mut server, &mut editor, KEY_DOWN);
    assert_eq!(settle(&mut server, &mut editor, RED), free);
}

#[test]
fn dragging_a_tiled_window_away_gives_back_its_earlier_size() {
    let mut server = server();
    let mut editor = app(&mut server, RED);
    chord(&mut server, &mut editor, KEY_LEFT);
    settle(&mut server, &mut editor, RED);

    server.move_pointer_to(at(64, 96), 0);
    server.press_button(BTN_LEFT, true, 0);
    roundtrip(&mut server, &mut [&mut editor]);
    editor.request_move(&mut server, 0);
    server.move_pointer_to(at(150, 120), 10);
    roundtrip(&mut server, &mut [&mut editor]);

    assert_eq!(settle(&mut server, &mut editor, RED).size, WINDOW.into());
    release(&mut server, &mut editor, 20);
}

#[test]
fn neighbouring_tiles_share_the_edge_that_is_dragged() {
    let mut server = server();
    let mut left = app(&mut server, RED);
    chord(&mut server, &mut left, KEY_LEFT);
    settle(&mut server, &mut left, RED);
    let mut right = app(&mut server, BLUE);
    chord(&mut server, &mut right, KEY_RIGHT);
    settle(&mut server, &mut right, BLUE);
    roundtrip(&mut server, &mut [&mut left, &mut right]);

    server.move_pointer_to(at(60, 96), 0);
    server.press_button(BTN_LEFT, true, 0);
    roundtrip(&mut server, &mut [&mut left, &mut right]);
    left.request_resize(&mut server, 0, ResizeEdge::Right);
    server.move_pointer_to(at(80, 96), 10);
    roundtrip(&mut server, &mut [&mut left, &mut right]);
    left.fill_configured_size(&mut server, 0, RED);
    right.fill_configured_size(&mut server, 0, BLUE);
    roundtrip(&mut server, &mut [&mut left, &mut right]);

    let geometries: Vec<Rect> = server
        .windows()
        .iter()
        .map(|window| window.geometry().expect("mapped"))
        .collect();
    assert!(geometries.contains(&rect(0, 0, 148, 192)), "{geometries:?}");
    assert!(
        geometries.contains(&rect(148, 0, 108, 192)),
        "{geometries:?}"
    );
    release(&mut server, &mut left, 20);
}

#[test]
fn with_reduce_motion_the_preview_fades_in_where_the_window_will_go() {
    let far_corner = |reduce: bool| {
        let mut server = server();
        server.set_reduce_motion(reduce);
        let mut renderer = Renderer::new().expect("renderer");
        let mut editor = app(&mut server, RED);
        grab(&mut server, &mut editor);
        server.move_pointer_to(at(0, 96), 1000);
        server.snap_tick(Duration::from_millis(1150));
        server.animate(Duration::from_millis(1150));
        server.animate(Duration::from_millis(1190));
        renderer.render(&mut server).expect("frame");
        let snapshot = renderer.snapshot(&server).expect("read back");
        let at = ((10 * snapshot.size.w + 100) * 4) as usize;
        snapshot.rgba[at..at + 3].to_vec()
    };

    let moving = far_corner(false);
    let fading = far_corner(true);

    assert!(
        moving
            .iter()
            .zip(BACKGROUND)
            .all(|(shown, below)| *shown <= below),
        "{moving:?}"
    );
    assert!(
        fading
            .iter()
            .zip(BACKGROUND)
            .all(|(shown, below)| *shown > below),
        "{fading:?}"
    );
}

#[test]
fn the_window_menu_can_tile_a_window_into_a_quarter_and_restore_it() {
    let mut server = server();
    let mut editor = app(&mut server, RED);
    let before = settle(&mut server, &mut editor, RED);

    editor.place_window(&mut server, 0, Placement::BottomRight);
    let quarter = settle(&mut server, &mut editor, RED);
    editor.place_window(&mut server, 0, Placement::Restore);
    let restored = settle(&mut server, &mut editor, RED);

    assert_eq!(quarter, rect(128, 96, 128, 96));
    assert_eq!(restored, before);
}
