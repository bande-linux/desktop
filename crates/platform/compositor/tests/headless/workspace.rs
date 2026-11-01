use std::time::Duration;

use crate::support::*;
use bande_compositor::{BACKGROUND, Compositor, Gesture, Renderer};
use bande_test_wayland::{BLUE, RED, TestClient, Workspace, roundtrip};

const WINDOW: (i32, i32) = (64, 48);

fn workspace_of(server: &Compositor, color_window: usize) -> Option<usize> {
    server.workspace_of(&server.windows()[color_window])
}

fn workspace_of_app(server: &Compositor, app_id: &str) -> Option<usize> {
    let window = server
        .windows()
        .iter()
        .find(|window| window.app_id() == app_id)?;
    server.workspace_of(window)
}

fn swipe(server: &mut Compositor, fingers: u32, steps: &[(f64, f64)], start: u32) -> u32 {
    server.gesture(Gesture::SwipeBegin { fingers }, start);
    let mut time = start;
    for &delta in steps {
        time += 10;
        server.gesture(
            Gesture::SwipeUpdate {
                delta: delta.into(),
            },
            time,
        );
    }
    time += 10;
    server.gesture(Gesture::SwipeEnd { cancelled: false }, time);
    time
}

#[test]
fn ctrl_super_shift_right_carries_the_window_to_a_new_workspace_and_alt_super_left_comes_back() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = window_of(&mut server, RED, WINDOW, EDITOR);
    let mut viewer = window_of(&mut server, BLUE, WINDOW, VIEWER);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    chord(
        &mut server,
        &[KEY_LEFTCTRL, KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_RIGHT],
    );
    let end = settle(&mut server, 0);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);
    let carried = (
        server.workspace_count(),
        server.active_workspace(),
        pixel(&mut renderer, &mut server, 128, 96),
        viewer.keyboard_focus_on(0),
    );

    chord(&mut server, &[KEY_LEFTALT, KEY_LEFTMETA, KEY_LEFT]);
    settle(&mut server, end);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    assert_eq!(carried, (2, 1, [0, 0, 0xff], true));
    assert_eq!(server.active_workspace(), 0);
    assert_eq!(pixel(&mut renderer, &mut server, 128, 96), [0xff, 0, 0]);
    assert!(editor.keyboard_focus_on(0));
    assert!(!viewer.keyboard_focus_on(0));
}

#[test]
fn a_workspace_left_empty_goes_away_once_the_switch_settles() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, WINDOW, EDITOR);
    let mut viewer = window_of(&mut server, BLUE, WINDOW, VIEWER);
    chord(
        &mut server,
        &[KEY_LEFTCTRL, KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_RIGHT],
    );
    let end = settle(&mut server, 0);

    chord(&mut server, &[KEY_LEFTALT, KEY_LEFTMETA, KEY_LEFT]);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);
    chord(
        &mut server,
        &[KEY_LEFTCTRL, KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_RIGHT],
    );
    let during = server.workspace_count();
    settle(&mut server, end);

    assert_eq!(during, 2);
    assert_eq!(server.workspace_count(), 1);
    assert_eq!(server.active_workspace(), 0);
}

#[test]
fn a_three_finger_swipe_tracks_the_fingers_and_a_short_one_springs_back() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = window_of(&mut server, RED, WINDOW, EDITOR);
    let mut viewer = window_of(&mut server, BLUE, WINDOW, VIEWER);
    chord(
        &mut server,
        &[KEY_LEFTCTRL, KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_RIGHT],
    );
    let end = settle(&mut server, 0);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    let time = swipe(&mut server, 3, &[(2.0, 0.0); 10], 1000);
    settle(&mut server, u64::from(time));
    let after_short = server.active_workspace();

    server.gesture(Gesture::SwipeBegin { fingers: 3 }, 2000);
    let mut halfway_through = None;
    for step in 1..=10 {
        server.gesture(
            Gesture::SwipeUpdate {
                delta: (35.0, 0.0).into(),
            },
            2000 + step * 10,
        );
        if step == 5 {
            halfway_through = Some(pixel(&mut renderer, &mut server, 128, 96));
        }
    }
    server.gesture(Gesture::SwipeEnd { cancelled: false }, 2110);
    settle(&mut server, end + 2110);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    assert_eq!(after_short, 1);
    assert_eq!(halfway_through, Some(BACKGROUND));
    assert_eq!(server.active_workspace(), 0);
    assert!(editor.keyboard_focus_on(0));
    assert!(editor.gestures().is_empty());
}

#[test]
fn the_taskbar_learns_the_workspaces_and_activating_an_app_switches_to_its_workspace() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, WINDOW, EDITOR);
    let mut viewer = window_of(&mut server, BLUE, WINDOW, VIEWER);
    let mut taskbar = TestClient::connect_as_shell(&mut server);
    taskbar.join_shell(&mut server);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer, &mut taskbar]);
    let at_start = (taskbar.workspaces(), workspace_of_app(&server, VIEWER));

    chord(
        &mut server,
        &[KEY_LEFTCTRL, KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_RIGHT],
    );
    let end = settle(&mut server, 0);
    chord(&mut server, &[KEY_LEFTALT, KEY_LEFTMETA, KEY_LEFT]);
    settle(&mut server, end);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer, &mut taskbar]);
    let apart = (taskbar.workspaces(), workspace_of_app(&server, VIEWER));

    taskbar.activate_app(&mut server, VIEWER);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer, &mut taskbar]);

    assert_eq!(at_start, (Some((1, 0)), Some(0)));
    assert_eq!(apart, (Some((2, 0)), Some(1)));
    assert_eq!(taskbar.workspaces(), Some((2, 1)));
    assert!(viewer.keyboard_focus_on(0));
}

#[test]
fn a_full_screen_window_gets_a_workspace_of_its_own_until_it_leaves_full_screen() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = window_of(&mut server, RED, WINDOW, EDITOR);
    let mut viewer = window_of(&mut server, BLUE, WINDOW, VIEWER);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    chord(&mut server, &[KEY_LEFTCTRL, KEY_LEFTMETA, KEY_F]);
    viewer.exchange(&mut server);
    viewer.fill_configured_size(&mut server, 0, BLUE);
    let end = settle(&mut server, 0);
    let full_screen = (
        server.workspace_count(),
        server.active_workspace(),
        workspace_of(&server, 1),
    );

    chord(&mut server, &[KEY_LEFTCTRL, KEY_LEFTMETA, KEY_F]);
    viewer.exchange(&mut server);
    viewer.fill_configured_size(&mut server, 0, BLUE);
    settle(&mut server, end);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    assert_eq!(full_screen, (2, 1, Some(1)));
    assert_eq!(server.workspace_count(), 1);
    assert_eq!(pixel(&mut renderer, &mut server, 128, 96), [0, 0, 0xff]);
    assert!(viewer.keyboard_focus_on(0));
}

#[test]
fn moving_a_window_from_its_menu_slides_it_away_and_hands_the_keyboard_on() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = window_of(&mut server, RED, WINDOW, EDITOR);
    let mut viewer = window_of(&mut server, BLUE, WINDOW, VIEWER);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    let end = settle(&mut server, 0);

    viewer.move_to_workspace(&mut server, 0, Workspace::Next);
    server.animate(Duration::from_millis(end + 16));
    let leaving = pixel(&mut renderer, &mut server, 162, 96);
    settle(&mut server, end + 16);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    assert!(leaving[2] > 0x80 && leaving[0] < 0x20, "{leaving:?}");
    assert_eq!(
        (server.workspace_count(), server.active_workspace()),
        (2, 0)
    );
    assert_eq!(pixel(&mut renderer, &mut server, 128, 96), [0xff, 0, 0]);
    assert!(editor.keyboard_focus_on(0));
}

#[test]
fn with_reduce_motion_a_switch_crossfades_in_place() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = window_of(&mut server, RED, WINDOW, EDITOR);
    let mut viewer = window_of(&mut server, BLUE, WINDOW, VIEWER);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);
    server.set_reduce_motion(true);

    chord(
        &mut server,
        &[KEY_LEFTCTRL, KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_RIGHT],
    );
    let end = settle(&mut server, 0);
    chord(&mut server, &[KEY_LEFTALT, KEY_LEFTMETA, KEY_LEFT]);
    server.animate(Duration::from_millis(end + 16));
    server.animate(Duration::from_millis(end + 40));
    let [red, green, blue] = pixel(&mut renderer, &mut server, 128, 96);

    assert!(
        red > 0x40 && blue > 0x40 && green < 0x28,
        "{red} {green} {blue}"
    );
}

#[test]
fn a_window_being_dragged_comes_along_and_the_workspace_it_left_empty_goes_away() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, WINDOW, EDITOR);
    let mut viewer = window_of(&mut server, BLUE, WINDOW, VIEWER);
    chord(
        &mut server,
        &[KEY_LEFTCTRL, KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_RIGHT],
    );
    let end = settle(&mut server, 0);
    chord(&mut server, &[KEY_LEFTALT, KEY_LEFTMETA, KEY_LEFT]);
    let end = settle(&mut server, end);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    server.move_pointer_to((128.0, 80.0).into(), 1);
    server.press_button(BTN_LEFT, true, 2);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);
    editor.request_move(&mut server, 0);
    server.move_pointer_to((140.0, 90.0).into(), 3);
    chord(&mut server, &[KEY_LEFTALT, KEY_LEFTMETA, KEY_RIGHT]);
    settle(&mut server, end);
    server.press_button(BTN_LEFT, false, 4);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    assert_eq!(server.workspace_count(), 1);
    assert!(
        server
            .windows()
            .iter()
            .all(|window| server.workspace_of(window) == Some(0))
    );
}
