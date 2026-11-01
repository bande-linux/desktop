use std::time::Duration;

use crate::support::*;
use bande_compositor::{BACKGROUND, Compositor, Renderer};
use bande_test_wayland::{BLUE, Placement, RED, ResizeEdge, TestClient, roundtrip};
use smithay::utils::{Logical, Rectangle};

const WINDOW: (i32, i32) = (64, 48);
const GRAB: (i32, i32) = (110, 80);

fn grab(server: &mut Compositor, client: &mut TestClient) {
    server.move_pointer_to(at(GRAB.0, GRAB.1), 0);
    server.press_button(BTN_LEFT, true, 0);
    roundtrip(server, &mut [client]);
    client.request_move(server, 0);
}

fn release(server: &mut Compositor, client: &mut TestClient, time: u32) {
    server.press_button(BTN_LEFT, false, time);
    roundtrip(server, &mut [client]);
}

fn drop_at(server: &mut Compositor, client: &mut TestClient, to: (i32, i32)) {
    grab(server, client);
    server.move_pointer_to(at(to.0, to.1), 1000);
    release(server, client, 1000);
}

fn lit(server: &mut Compositor, renderer: &mut Renderer, (x, y): (i32, i32), time: u64) -> bool {
    server.animate(Duration::from_millis(time));
    renderer.render(server).expect("frame");
    let snapshot = renderer.snapshot(server).expect("read back");
    let at = ((y * snapshot.size.w + x) * 4) as usize;
    snapshot.rgba[at..at + 3]
        .iter()
        .zip(BACKGROUND)
        .all(|(shown, below)| *shown > below)
}

fn settle(
    server: &mut Compositor,
    client: &mut TestClient,
    color: [u8; 4],
) -> Rectangle<i32, Logical> {
    if client
        .configured(0)
        .is_some_and(|configured| configured.width > 0)
    {
        client.fill_configured_size(server, 0, color);
    }
    roundtrip(server, &mut [client]);
    server.windows()[0].geometry().expect("mapped")
}

fn chord(server: &mut Compositor, client: &mut TestClient, key: u32) {
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
fn a_window_dragged_across_to_the_next_display_is_not_tiled() {
    let mut server = two_displays();
    let mut editor = window(&mut server, RED, WINDOW);

    grab(&mut server, &mut editor);
    server.move_pointer_to(at(255, 96), 1000);
    server.move_pointer_to(at(300, 96), 1100);
    release(&mut server, &mut editor, 1100);

    assert_eq!(settle(&mut server, &mut editor, RED).size, WINDOW.into());
    assert_eq!(server.output_of(&server.windows()[0]), SIDE);
}

#[test]
fn holding_at_a_shared_edge_for_250_ms_tiles_the_window() {
    let mut server = two_displays();
    let mut editor = window(&mut server, RED, WINDOW);

    grab(&mut server, &mut editor);
    server.move_pointer_to(at(255, 96), 1000);
    server.move_pointer_to(at(254, 99), 1100);
    server.move_pointer_to(at(255, 92), 1200);
    assert_eq!(
        server.snap_tick(Duration::from_millis(1200)),
        Some(Duration::from_millis(1250))
    );
    assert_eq!(server.snap_tick(Duration::from_millis(1250)), None);
    release(&mut server, &mut editor, 1250);

    assert_eq!(
        settle(&mut server, &mut editor, RED),
        rect(128, 0, 128, 192)
    );
}

#[test]
fn the_outer_edge_of_the_side_display_still_previews_at_once() {
    let mut server = two_displays();
    let mut editor = window(&mut server, RED, WINDOW);

    grab(&mut server, &mut editor);
    server.move_pointer_to(at(575, 120), 1000);
    assert_eq!(server.snap_tick(Duration::from_millis(1000)), None);
    release(&mut server, &mut editor, 1000);

    assert_eq!(
        settle(&mut server, &mut editor, RED),
        rect(256 + 160, 0, 160, 240)
    );
}

#[test]
fn a_window_dropped_at_an_edge_fills_the_half_quarter_or_screen_under_the_pointer() {
    for (to, expected) in [
        ((0, 96), rect(0, 0, 128, 192)),
        ((255, 4), rect(128, 0, 128, 96)),
        ((0, 190), rect(0, 96, 128, 96)),
        ((128, 0), rect(0, 0, 256, 192)),
    ] {
        let mut server = server();
        let mut editor = window(&mut server, RED, WINDOW);

        drop_at(&mut server, &mut editor, to);

        assert_eq!(settle(&mut server, &mut editor, RED), expected, "{to:?}");
    }
}

#[test]
fn a_window_dropped_over_the_left_rail_fills_the_half_beside_it() {
    for (to, expected) in [
        ((10, 96), Some(rect(BAR, 0, (256 - BAR) / 2, 192))),
        ((BAR, 96), Some(rect(BAR, 0, (256 - BAR) / 2, 192))),
        ((BAR + 8, 96), None),
    ] {
        let mut server = server();
        let mut shell = left_rail(&mut server);
        let mut editor = window(&mut server, RED, WINDOW);
        roundtrip(&mut server, &mut [&mut shell, &mut editor]);

        drop_at(&mut server, &mut editor, to);

        let geometry = settle(&mut server, &mut editor, RED);
        match expected {
            Some(tile) => assert_eq!(geometry, tile, "{to:?}"),
            None => assert_eq!(geometry.size, WINDOW.into(), "{to:?}"),
        }
    }
}

#[test]
fn the_preview_appears_as_soon_as_the_pointer_reaches_an_outer_edge() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = window(&mut server, RED, WINDOW);
    grab(&mut server, &mut editor);
    server.move_pointer_to(at(0, 96), 1000);

    assert_eq!(server.snap_tick(Duration::from_millis(1000)), None);
    lit(&mut server, &mut renderer, (60, 20), 1000);
    assert!(lit(&mut server, &mut renderer, (60, 20), 1100));

    release(&mut server, &mut editor, 1100);
    editor.fill_configured_size(&mut server, 0, RED);
    lit(&mut server, &mut renderer, (60, 20), 1200);
    server.animate(Duration::from_millis(4000));
    renderer.render(&mut server).expect("frame");
    let snapshot = renderer.snapshot(&server).expect("read back");
    let at = ((20 * snapshot.size.w + 60) * 4) as usize;
    assert_eq!(snapshot.rgba[at..at + 3], [0xff, 0, 0]);
}

#[test]
fn the_snap_preview_covers_its_tile_within_80_ms() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = window(&mut server, RED, WINDOW);
    grab(&mut server, &mut editor);
    server.move_pointer_to(at(0, 96), 1000);

    lit(&mut server, &mut renderer, (100, 10), 1000);
    assert!(lit(&mut server, &mut renderer, (100, 10), 1080));
}

#[test]
fn ctrl_super_arrows_walk_through_halves_quarters_zoom_and_back() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);
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
    let mut editor = window(&mut server, RED, WINDOW);
    let free = settle(&mut server, &mut editor, RED);

    chord(&mut server, &mut editor, KEY_UP);
    assert_eq!(settle(&mut server, &mut editor, RED), rect(0, 0, 256, 192));
    chord(&mut server, &mut editor, KEY_DOWN);
    assert_eq!(settle(&mut server, &mut editor, RED), free);
}

#[test]
fn neighbouring_tiles_share_the_edge_that_is_dragged() {
    let mut server = server();
    let mut left = window(&mut server, RED, WINDOW);
    chord(&mut server, &mut left, KEY_LEFT);
    settle(&mut server, &mut left, RED);
    let mut right = window(&mut server, BLUE, WINDOW);
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

    let geometries: Vec<Rectangle<i32, Logical>> = server
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
        let mut editor = window(&mut server, RED, WINDOW);
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
    let mut editor = window(&mut server, RED, WINDOW);
    let before = settle(&mut server, &mut editor, RED);

    editor.place_window(&mut server, 0, Placement::BottomRight);
    let quarter = settle(&mut server, &mut editor, RED);
    editor.place_window(&mut server, 0, Placement::Restore);
    let restored = settle(&mut server, &mut editor, RED);

    assert_eq!(quarter, rect(128, 96, 128, 96));
    assert_eq!(restored, before);
}

fn escape(server: &mut Compositor, client: &mut TestClient) {
    server.key(KEY_ESC, true, 1200);
    server.key(KEY_ESC, false, 1201);
    roundtrip(server, &mut [client]);
}

#[test]
fn escape_while_the_snap_preview_shows_puts_the_window_back_untiled() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);
    let before = settle(&mut server, &mut editor, RED);
    grab(&mut server, &mut editor);
    server.move_pointer_to(at(0, 96), 1000);
    server.snap_tick(Duration::from_millis(1150));

    escape(&mut server, &mut editor);
    release(&mut server, &mut editor, 1300);

    assert_eq!(settle(&mut server, &mut editor, RED), before);
    assert!(editor.keys().is_empty());
}

#[test]
fn escape_while_dragging_a_tiled_window_returns_it_to_its_tile() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);
    chord(&mut server, &mut editor, KEY_LEFT);
    let tiled = settle(&mut server, &mut editor, RED);
    assert_eq!(tiled, rect(0, 0, 128, 192));
    server.move_pointer_to(at(64, 96), 0);
    server.press_button(BTN_LEFT, true, 0);
    roundtrip(&mut server, &mut [&mut editor]);
    editor.request_move(&mut server, 0);
    server.move_pointer_to(at(150, 120), 10);
    roundtrip(&mut server, &mut [&mut editor]);
    assert_eq!(settle(&mut server, &mut editor, RED).size, WINDOW.into());

    escape(&mut server, &mut editor);
    release(&mut server, &mut editor, 1300);

    assert_eq!(settle(&mut server, &mut editor, RED), tiled);
    assert!(!editor.keys().iter().any(|(key, _)| *key == KEY_ESC));
}
