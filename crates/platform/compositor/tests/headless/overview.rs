use std::time::Duration;

use crate::support::*;
use unbound_compositor::{BACKGROUND, Compositor, Gesture, Renderer, Snapshot};
use unbound_test_wayland::{BLUE, RED, TestClient, roundtrip};

const WINDOW: (i32, i32) = (64, 48);
const ADD_WORKSPACE: (f64, f64) = (151.0, 25.0);
const BELOW_THE_STRIP: usize = 50;

fn app(server: &mut Compositor, color: [u8; 4], app_id: &str) -> TestClient {
    window_of(server, color, WINDOW, app_id)
}

fn open_with_keys(server: &mut Compositor) {
    server.key(KEY_LEFTALT, true, 0);
    server.key(KEY_LEFTMETA, true, 0);
    server.key(KEY_UP, true, 0);
    server.key(KEY_UP, false, 0);
    server.key(KEY_LEFTMETA, false, 0);
    server.key(KEY_LEFTALT, false, 0);
}

fn snapshot(renderer: &mut Renderer, server: &mut Compositor) -> Snapshot {
    renderer.render(server).expect("frame");
    renderer.snapshot(server).expect("read back")
}

fn find(snapshot: &Snapshot, color: [u8; 3]) -> Option<(f64, f64)> {
    snapshot
        .rgba
        .chunks(4)
        .enumerate()
        .find_map(|(index, pixel)| {
            let width = snapshot.size.w as usize;
            (pixel[..3] == color && index / width > BELOW_THE_STRIP)
                .then(|| ((index % width) as f64 + 4.0, (index / width) as f64 + 4.0))
        })
}

fn click(server: &mut Compositor, (x, y): (f64, f64), time: u32) {
    server.move_pointer_to((x, y).into(), time);
    server.press_button(BTN_LEFT, true, time);
    server.press_button(BTN_LEFT, false, time + 1);
}

#[test]
fn the_overview_spreads_overlapping_windows_and_a_click_picks_one() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = app(&mut server, RED, EDITOR);
    let mut viewer = app(&mut server, BLUE, VIEWER);
    let end = settle(&mut server, 0);
    let before = snapshot(&mut renderer, &mut server);

    open_with_keys(&mut server);
    let end = settle(&mut server, end);
    let spread = snapshot(&mut renderer, &mut server);
    let red = find(&spread, [0xff, 0, 0]).expect("the editor is shown");
    click(&mut server, red, 10);
    settle(&mut server, end);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    assert!(find(&before, [0xff, 0, 0]).is_none());
    assert!(find(&spread, [0, 0, 0xff]).is_some());
    assert!(!server.overview_shown());
    assert!(editor.keyboard_focus_on(0));
    assert!(editor.buttons().is_empty());
}

#[test]
fn a_middle_click_in_the_overview_closes_that_window_and_keeps_the_overview_open() {
    const BTN_MIDDLE: u32 = 0x112;
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = app(&mut server, RED, EDITOR);
    let mut viewer = app(&mut server, BLUE, VIEWER);
    let end = settle(&mut server, 0);

    open_with_keys(&mut server);
    settle(&mut server, end);
    let spread = snapshot(&mut renderer, &mut server);
    let red = find(&spread, [0xff, 0, 0]).expect("the editor is shown");
    server.move_pointer_to(red.into(), 10);
    server.press_button(BTN_MIDDLE, true, 10);
    server.press_button(BTN_MIDDLE, false, 11);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    assert!(editor.close_requested(0));
    assert!(!viewer.close_requested(0));
    assert!(server.overview_shown());
    assert!(editor.buttons().is_empty());
}

#[test]
fn keys_typed_in_the_overview_stay_out_of_the_app_and_escape_closes_it() {
    let mut server = server();
    let mut editor = app(&mut server, RED, EDITOR);
    let end = settle(&mut server, 0);
    open_with_keys(&mut server);
    settle(&mut server, end);

    for key in [KEY_A, KEY_ESC, KEY_ESC] {
        server.key(key, true, 0);
        server.key(key, false, 0);
    }
    roundtrip(&mut server, &mut [&mut editor]);

    assert!(!server.overview_shown());
    assert!(editor.keyboard_focus_on(0));
    assert!(
        editor
            .keys()
            .iter()
            .all(|(key, _)| ![KEY_A, KEY_ESC].contains(key))
    );
}

#[test]
fn a_three_finger_swipe_up_opens_the_overview_and_down_closes_it() {
    let mut server = server();
    let _editor = app(&mut server, RED, EDITOR);
    let swipe = |server: &mut Compositor, dy: f64, start: u32| {
        server.gesture(Gesture::SwipeBegin { fingers: 3 }, start);
        for step in 1..=10 {
            server.gesture(
                Gesture::SwipeUpdate {
                    delta: (0.0, dy).into(),
                },
                start + step * 10,
            );
        }
        server.gesture(Gesture::SwipeEnd { cancelled: false }, start + 110);
    };

    swipe(&mut server, -30.0, 100);
    let end = settle(&mut server, 210);
    let opened = server.overview_shown();
    swipe(&mut server, 30.0, 1000);
    settle(&mut server, end + 1000);

    assert!(opened);
    assert!(!server.overview_shown());
}

#[test]
fn dragging_a_window_onto_the_plus_moves_it_to_a_new_workspace() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = app(&mut server, RED, EDITOR);
    let mut viewer = app(&mut server, BLUE, VIEWER);
    let end = settle(&mut server, 0);
    open_with_keys(&mut server);
    let end = settle(&mut server, end);
    let (x, y) = find(&snapshot(&mut renderer, &mut server), [0, 0, 0xff]).expect("viewer");

    server.move_pointer_to((x, y).into(), 1);
    server.press_button(BTN_LEFT, true, 2);
    server.move_pointer_to((x + 10.0, y - 10.0).into(), 3);
    server.move_pointer_to(ADD_WORKSPACE.into(), 4);
    server.press_button(BTN_LEFT, false, 5);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);
    let moved = server
        .windows()
        .iter()
        .map(|window| server.workspace_of(window))
        .collect::<Vec<_>>();
    open_with_keys(&mut server);
    settle(&mut server, end);

    assert_eq!(server.workspace_count(), 2);
    assert!(moved.contains(&Some(0)) && moved.contains(&Some(1)));
    assert_eq!(server.active_workspace(), 0);
    assert!(editor.keyboard_focus_on(0));
}

#[test]
fn the_dock_can_show_the_windows_of_one_app() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = app(&mut server, RED, EDITOR);
    let mut viewer = app(&mut server, BLUE, VIEWER);
    let mut dock = TestClient::connect_as_shell(&mut server);
    dock.join_shell(&mut server);
    let end = settle(&mut server, 0);

    dock.show_windows(&mut server, EDITOR);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer, &mut dock]);
    settle(&mut server, end);
    let shown = snapshot(&mut renderer, &mut server);

    assert!(server.overview_shown());
    assert!(find(&shown, [0xff, 0, 0]).is_some());
    assert!(find(&shown, [0, 0, 0xff]).is_none());
}

#[test]
fn the_selected_window_shows_its_title_below_it() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let _editor = app(&mut server, RED, EDITOR);
    let end = settle(&mut server, 0);
    let below = rgb_at(&snapshot(&mut renderer, &mut server), 128, 162);

    open_with_keys(&mut server);
    settle(&mut server, end);
    let titled = rgb_at(&snapshot(&mut renderer, &mut server), 128, 162);

    assert_eq!(below, BACKGROUND);
    assert_ne!(titled, BACKGROUND);
}

#[test]
fn with_reduce_motion_the_overview_fades_instead_of_moving_windows() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let _editor = app(&mut server, RED, EDITOR);
    let end = settle(&mut server, 0);
    server.set_reduce_motion(true);

    open_with_keys(&mut server);
    server.animate(Duration::from_millis(end + 16));
    server.animate(Duration::from_millis(end + 60));
    let halfway = snapshot(&mut renderer, &mut server);
    let [old_red, _, _] = rgb_at(&halfway, 128, 80);
    let [new_red, _, _] = rgb_at(&halfway, 128, 128);

    assert!(old_red > 0x40 && old_red < 0xf0, "{old_red}");
    assert!(new_red > 0x40 && new_red < 0xf0, "{new_red}");
}
