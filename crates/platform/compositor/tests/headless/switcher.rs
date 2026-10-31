use std::time::Duration;

use crate::support::*;
use smithay::utils::Size;
use unbound_compositor::{Compositor, Renderer};
use unbound_test_wayland::{BLUE, GREEN, RED, TestClient, roundtrip};

const OUTPUT: (i32, i32) = (480, 270);
const WINDOW: (i32, i32) = (160, 100);
const KEY_GRAVE: u32 = 41;
const PLAYER: &str = "org.example.Player";

fn server() -> Compositor {
    Compositor::new(Size::from(OUTPUT)).expect("headless compositor")
}

fn another_window(server: &mut Compositor, client: &mut TestClient, color: [u8; 4], index: usize) {
    client.show_window(server, &xrgb(&solid(color, WINDOW), WINDOW));
    client.set_app_id(server, index, EDITOR);
}

fn hold_super_and_tap(server: &mut Compositor, key: u32) {
    server.key(KEY_LEFTMETA, true, 0);
    tap(server, key);
}

fn run_for(server: &mut Compositor, from: u64, millis: u64) -> u64 {
    let mut at = from;
    while at < from + millis {
        at += 16;
        server.animate(Duration::from_millis(at));
    }
    at
}

#[test]
fn a_quick_super_tab_switches_to_the_last_app_without_showing_the_card() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, WINDOW, EDITOR);
    let mut viewer = window_of(&mut server, BLUE, WINDOW, VIEWER);
    let end = settle(&mut server, 0);

    hold_super_and_tap(&mut server, KEY_TAB);
    server.key(KEY_LEFTMETA, false, 0);
    settle(&mut server, end);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    assert!(!server.switcher_shown());
    assert!(editor.keyboard_focus_on(0));
    assert!(!editor.keys().iter().any(|(key, _)| *key == KEY_TAB));
}

#[test]
fn holding_super_shows_the_card_after_a_short_dwell_and_release_picks_the_selection() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, WINDOW, EDITOR);
    let mut viewer = window_of(&mut server, GREEN, WINDOW, VIEWER);
    let mut player = window_of(&mut server, BLUE, WINDOW, PLAYER);
    let end = settle(&mut server, 0);

    hold_super_and_tap(&mut server, KEY_TAB);
    let early = run_for(&mut server, end, 100);
    let shown_early = server.switcher_shown();
    let late = run_for(&mut server, early, 100);
    let shown_late = server.switcher_shown();
    tap(&mut server, KEY_TAB);
    server.key(KEY_LEFTMETA, false, 0);
    settle(&mut server, late);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer, &mut player]);

    assert!(!shown_early);
    assert!(shown_late);
    assert!(!server.switcher_shown());
    assert!(editor.keyboard_focus_on(0));
}

#[test]
fn shift_tab_steps_backwards_and_escape_keeps_the_current_app() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, WINDOW, EDITOR);
    let mut viewer = window_of(&mut server, GREEN, WINDOW, VIEWER);
    let mut player = window_of(&mut server, BLUE, WINDOW, PLAYER);
    settle(&mut server, 0);

    server.key(KEY_LEFTMETA, true, 0);
    server.key(KEY_LEFTSHIFT, true, 0);
    tap(&mut server, KEY_TAB);
    server.key(KEY_LEFTSHIFT, false, 0);
    server.key(KEY_LEFTMETA, false, 0);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer, &mut player]);
    let backwards = editor.keyboard_focus_on(0);

    hold_super_and_tap(&mut server, KEY_TAB);
    tap(&mut server, KEY_ESC);
    server.key(KEY_LEFTMETA, false, 0);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer, &mut player]);

    assert!(backwards);
    assert!(editor.keyboard_focus_on(0));
    assert!(!editor.keys().iter().any(|(key, _)| *key == KEY_ESC));
}

#[test]
fn q_in_the_switcher_quits_the_selected_app_and_h_hides_it() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, WINDOW, EDITOR);
    let mut viewer = window_of(&mut server, GREEN, WINDOW, VIEWER);
    let mut player = window_of(&mut server, BLUE, WINDOW, PLAYER);
    settle(&mut server, 0);

    hold_super_and_tap(&mut server, KEY_TAB);
    tap(&mut server, KEY_Q);
    tap(&mut server, KEY_H);
    server.key(KEY_LEFTMETA, false, 0);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer, &mut player]);

    assert!(viewer.close_requested(0));
    assert!(!editor.close_requested(0) && !player.close_requested(0));
    assert!(player.keyboard_focus_on(0));
    let shown = server
        .windows()
        .iter()
        .filter(|window| window.is_hidden())
        .count();
    assert_eq!(shown, 1);
}

#[test]
fn super_grave_cycles_the_focused_apps_windows_at_once() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, WINDOW, EDITOR);
    another_window(&mut server, &mut editor, GREEN, 1);
    another_window(&mut server, &mut editor, BLUE, 2);
    let mut viewer = window_of(&mut server, BLUE, WINDOW, VIEWER);
    let end = settle(&mut server, 0);
    hold_super_and_tap(&mut server, KEY_TAB);
    server.key(KEY_LEFTMETA, false, 0);
    let end = settle(&mut server, end);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);
    let started_on_last = editor.keyboard_focus_on(2);

    hold_super_and_tap(&mut server, KEY_GRAVE);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);
    let first_step = editor.keyboard_focus_on(1);
    tap(&mut server, KEY_GRAVE);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);
    let second_step = editor.keyboard_focus_on(0);
    run_for(&mut server, end, 200);
    let strip = server.switcher_shown();
    server.key(KEY_LEFTMETA, false, 0);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    assert!(started_on_last && first_step && second_step && strip);
    assert!(editor.keyboard_focus_on(0));
    assert!(!server.switcher_shown());
}

#[test]
fn down_in_the_switcher_lists_the_apps_windows_and_release_picks_one() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, WINDOW, EDITOR);
    another_window(&mut server, &mut editor, GREEN, 1);
    let mut viewer = window_of(&mut server, BLUE, WINDOW, VIEWER);
    settle(&mut server, 0);

    hold_super_and_tap(&mut server, KEY_TAB);
    tap(&mut server, KEY_DOWN);
    let listed = server.switcher_shown();
    tap(&mut server, KEY_TAB);
    server.key(KEY_LEFTMETA, false, 0);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    assert!(listed);
    assert!(editor.keyboard_focus_on(0));
}

#[test]
fn clicking_an_icon_switches_to_its_app() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, WINDOW, EDITOR);
    let mut viewer = window_of(&mut server, GREEN, WINDOW, VIEWER);
    let mut player = window_of(&mut server, BLUE, WINDOW, PLAYER);
    let end = settle(&mut server, 0);

    hold_super_and_tap(&mut server, KEY_TAB);
    run_for(&mut server, end, 200);
    let (x, y) = server
        .switcher_tiles()
        .get(2)
        .copied()
        .expect("a tile for the third app");
    server.move_pointer_to((x, y).into(), 1);
    server.press_button(BTN_LEFT, true, 2);
    server.press_button(BTN_LEFT, false, 3);
    server.key(KEY_LEFTMETA, false, 4);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer, &mut player]);

    assert!(editor.keyboard_focus_on(0));
    assert!(editor.buttons().is_empty() && player.buttons().is_empty());
}

fn check_golden(name: &str, renderer: &mut Renderer, server: &mut Compositor) {
    renderer.render(server).expect("frame");
    let snapshot = renderer.snapshot(server).expect("read back");
    unbound_golden::assert_golden!(
        name,
        &unbound_golden::from_rgba(
            snapshot.size.w.unsigned_abs(),
            snapshot.size.h.unsigned_abs(),
            snapshot.rgba,
        )
    );
}

#[test]
fn the_app_switcher_matches_its_goldens_at_both_scales() {
    for (scale, name) in [(1.0, "switcher-light"), (1.5, "switcher-light-1.5x")] {
        let mut server = server();
        server.set_output_scale(scale);
        let mut renderer = Renderer::new().expect("renderer");
        let _editor = window_of(&mut server, RED, WINDOW, EDITOR);
        let _viewer = window_of(&mut server, GREEN, WINDOW, VIEWER);
        let _player = window_of(&mut server, BLUE, WINDOW, PLAYER);
        let end = settle(&mut server, 0);
        server.move_pointer_to((470.0, 260.0).into(), 0);

        hold_super_and_tap(&mut server, KEY_TAB);
        settle(&mut server, end);

        check_golden(name, &mut renderer, &mut server);
    }
}

#[test]
fn the_window_list_matches_its_golden() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = window_of(&mut server, RED, WINDOW, EDITOR);
    another_window(&mut server, &mut editor, GREEN, 1);
    let end = settle(&mut server, 0);
    server.move_pointer_to((470.0, 260.0).into(), 0);

    hold_super_and_tap(&mut server, KEY_GRAVE);
    settle(&mut server, end);

    check_golden("switcher-windows", &mut renderer, &mut server);
}
