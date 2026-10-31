use crate::support::*;
use smithay::utils::Size;
use unbound_compositor::{Compositor, Rect, Renderer};
use unbound_test_wayland::{BLUE, GREEN, RED, roundtrip};

const OUTPUT: (i32, i32) = (480, 270);
const WINDOW: (i32, i32) = (160, 100);
const KEY_ENTER: u32 = 28;

fn server() -> Compositor {
    Compositor::new(Size::from(OUTPUT)).expect("headless compositor")
}

fn tile_with_keys(server: &mut Compositor, arrow: u32) {
    server.key(KEY_LEFTCTRL, true, 0);
    server.key(KEY_LEFTMETA, true, 0);
    server.key(arrow, true, 0);
    server.key(arrow, false, 0);
    server.key(KEY_LEFTMETA, false, 0);
    server.key(KEY_LEFTCTRL, false, 0);
}

fn tile_of(server: &Compositor, nth: usize) -> Option<Rect> {
    server.windows()[nth].tile()
}

#[test]
fn tiling_a_window_to_a_half_offers_the_others_for_the_other_half() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);
    let mut viewer = window(&mut server, BLUE, WINDOW);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    tile_with_keys(&mut server, KEY_LEFT);
    let offered = server.other_half_offered();
    tap(&mut server, KEY_ENTER);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    assert!(offered);
    assert!(!server.other_half_offered());
    let left = tile_of(&server, 0).expect("the viewer is tiled to the left");
    let right = tile_of(&server, 1).expect("the editor fills the right");
    assert_eq!(left.location.x + left.size.w, right.location.x);
    assert!(editor.keyboard_focus_on(0));
}

#[test]
fn escape_dismisses_the_offer_without_typing_into_the_app() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);
    let mut viewer = window(&mut server, BLUE, WINDOW);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    tile_with_keys(&mut server, KEY_RIGHT);
    tap(&mut server, KEY_A);
    tap(&mut server, KEY_ESC);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);
    tap(&mut server, KEY_A);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    assert!(!server.other_half_offered());
    assert!(
        server
            .windows()
            .iter()
            .filter(|window| window.tile().is_some())
            .count()
            == 1
    );
    let typed: Vec<(u32, bool)> = viewer
        .keys()
        .iter()
        .copied()
        .filter(|(key, _)| [KEY_A, KEY_ESC].contains(key))
        .collect();
    assert_eq!(typed, [(KEY_A, true), (KEY_A, false)]);
}

#[test]
fn clicking_a_thumbnail_fills_the_other_half_with_it() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);
    let mut viewer = window(&mut server, BLUE, WINDOW);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    tile_with_keys(&mut server, KEY_LEFT);
    settle(&mut server, 0);
    server.move_pointer_to(at(OUTPUT.0 * 3 / 4, OUTPUT.1 / 2), 1);
    server.press_button(BTN_LEFT, true, 2);
    server.press_button(BTN_LEFT, false, 3);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    assert!(tile_of(&server, 1).is_some());
    assert!(editor.keyboard_focus_on(0));
    assert!(editor.buttons().is_empty());
}

#[test]
fn a_lone_window_is_tiled_without_an_offer() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);
    roundtrip(&mut server, &mut [&mut editor]);

    tile_with_keys(&mut server, KEY_LEFT);

    assert!(!server.other_half_offered());
}

#[test]
fn the_offer_matches_its_golden() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = window(&mut server, RED, WINDOW);
    let mut player = window(&mut server, GREEN, WINDOW);
    let mut viewer = window(&mut server, BLUE, WINDOW);
    roundtrip(&mut server, &mut [&mut editor, &mut player, &mut viewer]);
    server.move_pointer_to(at(5, 5), 0);

    tile_with_keys(&mut server, KEY_LEFT);
    roundtrip(&mut server, &mut [&mut editor, &mut player, &mut viewer]);
    viewer.fill_configured_size(&mut server, 0, BLUE);
    roundtrip(&mut server, &mut [&mut editor, &mut player, &mut viewer]);
    settle(&mut server, 0);

    renderer.render(&mut server).expect("frame");
    let snapshot = renderer.snapshot(&server).expect("read back");
    unbound_golden::assert_golden_rgba!(
        "other-half",
        (
            snapshot.size.w.unsigned_abs(),
            snapshot.size.h.unsigned_abs()
        ),
        snapshot.rgba
    );
}
