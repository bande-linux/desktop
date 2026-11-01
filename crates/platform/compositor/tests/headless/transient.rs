use std::time::Duration;

use crate::support::*;
use bande_compositor::{Compositor, Renderer};
use bande_test_wayland::{BLUE, GREEN, RED, TestClient, roundtrip};
use smithay::utils::{Logical, Point, Rectangle, Size};

const OUTPUT: (i32, i32) = (480, 270);
const PARENT: (i32, i32) = (200, 120);
const DIALOG: (i32, i32) = (80, 40);

fn server() -> Compositor {
    Compositor::new(Size::from(OUTPUT)).expect("headless compositor")
}

fn app_with_dialog(server: &mut Compositor) -> TestClient {
    let mut client = window(server, RED, PARENT);
    client.show_child_window(server, &xrgb(&solid(GREEN, DIALOG), DIALOG), 0);
    client
}

fn other_app(server: &mut Compositor) -> TestClient {
    window(server, BLUE, (60, 60))
}

fn rect_of(server: &Compositor, width: i32) -> Rectangle<i32, Logical> {
    server
        .windows()
        .iter()
        .find(|window| window.size().is_some_and(|size| size.w == width))
        .and_then(|window| window.geometry())
        .expect("a mapped window of that width")
}

fn stacking(server: &Compositor) -> Vec<i32> {
    server
        .windows()
        .iter()
        .map(|window| window.size().expect("mapped").w)
        .collect()
}

fn shown(server: &Compositor) -> Vec<i32> {
    server
        .windows()
        .iter()
        .filter(|window| !window.is_minimized())
        .map(|window| window.size().expect("mapped").w)
        .collect()
}

fn beside_dialog(server: &Compositor) -> (i32, i32) {
    let parent = rect_of(server, PARENT.0);
    (parent.loc.x + 10, parent.loc.y + 10)
}

#[test]
fn a_dialog_opens_centred_over_its_parent_after_the_parent_moved() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    client.use_seat(&mut server);
    client.show_window(&mut server, &xrgb(&solid(RED, PARENT), PARENT));
    let parent = rect_of(&server, PARENT.0);
    server.move_pointer_to(at(parent.loc.x + 5, parent.loc.y + 5), 0);
    server.press_button(BTN_LEFT, true, 0);
    roundtrip(&mut server, &mut [&mut client]);
    client.request_move(&mut server, 0);
    server.move_pointer_to(at(parent.loc.x - 95, parent.loc.y + 45), 1);
    server.press_button(BTN_LEFT, false, 2);
    roundtrip(&mut server, &mut [&mut client]);

    client.show_child_window(&mut server, &xrgb(&solid(GREEN, DIALOG), DIALOG), 0);
    roundtrip(&mut server, &mut [&mut client]);

    let parent = rect_of(&server, PARENT.0);
    let dialog = rect_of(&server, DIALOG.0);
    assert_eq!(
        dialog.loc,
        (
            parent.loc.x + (PARENT.0 - DIALOG.0) / 2,
            parent.loc.y + (PARENT.1 - DIALOG.1) / 2
        )
            .into()
    );
    assert!(client.keyboard_focus_on(1));
}

#[test]
fn activating_the_parent_brings_its_dialog_along_on_top() {
    let mut server = server();
    let mut editor = app_with_dialog(&mut server);
    let mut other = other_app(&mut server);
    roundtrip(&mut server, &mut [&mut editor, &mut other]);
    assert_eq!(stacking(&server), [PARENT.0, DIALOG.0, 60]);

    let (x, y) = beside_dialog(&server);
    click(&mut server, x, y);
    roundtrip(&mut server, &mut [&mut editor, &mut other]);

    assert_eq!(stacking(&server), [60, PARENT.0, DIALOG.0]);
    assert!(editor.keyboard_focus_on(0));
}

#[test]
fn a_dialog_of_a_background_app_opens_directly_above_its_parent() {
    let mut server = server();
    let mut editor = TestClient::connect(&mut server);
    editor.use_seat(&mut server);
    editor.show_window(&mut server, &xrgb(&solid(RED, PARENT), PARENT));
    let mut other = other_app(&mut server);
    roundtrip(&mut server, &mut [&mut editor, &mut other]);

    editor.show_child_window(&mut server, &xrgb(&solid(GREEN, DIALOG), DIALOG), 0);
    roundtrip(&mut server, &mut [&mut editor, &mut other]);

    assert_eq!(stacking(&server), [PARENT.0, DIALOG.0, 60]);
    assert!(other.keyboard_focus_on(0));
}

#[test]
fn moving_the_parent_carries_its_dialog() {
    let mut server = server();
    let mut editor = app_with_dialog(&mut server);
    let before = rect_of(&server, DIALOG.0).loc;
    let (x, y) = beside_dialog(&server);

    server.move_pointer_to(at(x, y), 0);
    server.press_button(BTN_LEFT, true, 0);
    roundtrip(&mut server, &mut [&mut editor]);
    editor.request_move(&mut server, 0);
    server.move_pointer_to(at(x + 40, y + 25), 1);
    roundtrip(&mut server, &mut [&mut editor]);

    assert_eq!(
        rect_of(&server, DIALOG.0).loc,
        before + Point::from((40, 25))
    );
}

#[test]
fn minimizing_a_dialog_minimizes_its_parent_and_restoring_the_app_brings_both_back() {
    let mut server = server();
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_shell(&mut server);
    let mut editor = app_with_dialog(&mut server);
    editor.set_app_id(&mut server, 0, EDITOR);
    editor.set_app_id(&mut server, 1, EDITOR);
    let mut other = other_app(&mut server);
    shell.activate_app(&mut server, EDITOR);
    roundtrip(&mut server, &mut [&mut shell, &mut editor, &mut other]);
    assert!(editor.keyboard_focus_on(1));

    editor.set_minimized(&mut server, 1);
    roundtrip(&mut server, &mut [&mut shell, &mut editor, &mut other]);
    assert_eq!(shown(&server), [60]);
    assert!(!editor.keyboard_focus_on(0) && !editor.keyboard_focus_on(1));

    shell.activate_app(&mut server, EDITOR);
    roundtrip(&mut server, &mut [&mut shell, &mut editor, &mut other]);
    assert_eq!(shown(&server), [60, PARENT.0, DIALOG.0]);
    assert!(editor.keyboard_focus_on(1));
}

#[test]
fn super_h_on_the_parent_takes_its_dialog_along() {
    let mut server = server();
    let mut editor = app_with_dialog(&mut server);
    let mut other = other_app(&mut server);
    let (x, y) = beside_dialog(&server);
    click(&mut server, x, y);
    roundtrip(&mut server, &mut [&mut editor, &mut other]);

    server.key(KEY_LEFTMETA, true, 0);
    server.key(KEY_H, true, 0);
    server.key(KEY_H, false, 0);
    server.key(KEY_LEFTMETA, false, 0);
    roundtrip(&mut server, &mut [&mut editor, &mut other]);

    let minimized: Vec<bool> = server
        .windows()
        .iter()
        .map(|window| window.is_minimized())
        .collect();
    assert_eq!(minimized.iter().filter(|minimized| **minimized).count(), 2);
    assert!(!editor.keyboard_focus_on(0) && !editor.keyboard_focus_on(1));
}

#[test]
fn a_modal_dialog_takes_clicks_on_its_parent_and_shakes() {
    let mut server = server();
    let mut editor = app_with_dialog(&mut server);
    editor.make_modal(&mut server, 1);
    let mut other = other_app(&mut server);
    roundtrip(&mut server, &mut [&mut editor, &mut other]);
    let dialog = rect_of(&server, DIALOG.0);
    let buttons = editor.buttons().len();

    let (x, y) = beside_dialog(&server);
    click(&mut server, x, y);
    roundtrip(&mut server, &mut [&mut editor, &mut other]);

    assert_eq!(editor.buttons().len(), buttons);
    assert!(editor.keyboard_focus_on(1));
    assert_eq!(stacking(&server), [60, PARENT.0, DIALOG.0]);
    server.animate(Duration::from_millis(16));
    assert!(server.is_animating());

    click(&mut server, dialog.loc.x + 5, dialog.loc.y + 5);
    roundtrip(&mut server, &mut [&mut editor, &mut other]);
    assert!(editor.buttons().len() > buttons);
}

#[test]
fn a_non_modal_dialog_lets_its_parent_take_clicks() {
    let mut server = server();
    let mut editor = app_with_dialog(&mut server);
    let buttons = editor.buttons().len();

    let (x, y) = beside_dialog(&server);
    click(&mut server, x, y);
    roundtrip(&mut server, &mut [&mut editor]);

    assert!(editor.buttons().len() > buttons);
    assert!(editor.keyboard_focus_on(0));
}

#[test]
fn moving_a_dialog_to_the_next_workspace_takes_its_parent_along() {
    let mut server = server();
    let mut editor = app_with_dialog(&mut server);
    let mut other = other_app(&mut server);
    let (x, y) = beside_dialog(&server);
    click(&mut server, x, y);
    let dialog = rect_of(&server, DIALOG.0);
    click(&mut server, dialog.loc.x + 5, dialog.loc.y + 5);
    roundtrip(&mut server, &mut [&mut editor, &mut other]);
    assert!(editor.keyboard_focus_on(1));

    for key in [KEY_LEFTCTRL, KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_RIGHT] {
        server.key(key, true, 0);
    }
    for key in [KEY_RIGHT, KEY_LEFTSHIFT, KEY_LEFTMETA, KEY_LEFTCTRL] {
        server.key(key, false, 0);
    }
    settle(&mut server, 0);
    roundtrip(&mut server, &mut [&mut editor, &mut other]);

    let workspaces: Vec<(i32, Option<usize>)> = server
        .windows()
        .iter()
        .map(|window| {
            (
                window.size().expect("mapped").w,
                server.workspace_of(window),
            )
        })
        .collect();
    assert!(workspaces.contains(&(PARENT.0, Some(1))));
    assert!(workspaces.contains(&(DIALOG.0, Some(1))));
    assert!(workspaces.contains(&(60, Some(0))));
}

#[test]
fn the_app_switcher_offers_an_app_once_even_when_its_dialog_has_no_app_id() {
    let mut server = server();
    let mut editor = app_with_dialog(&mut server);
    editor.set_app_id(&mut server, 0, EDITOR);
    let mut other = other_app(&mut server);
    roundtrip(&mut server, &mut [&mut editor, &mut other]);
    let end = settle(&mut server, 0);

    server.key(KEY_LEFTMETA, true, 0);
    server.key(KEY_TAB, true, 0);
    server.key(KEY_TAB, false, 0);
    for step in 1..20 {
        server.animate(Duration::from_millis(end + step * 16));
    }

    assert!(server.switcher_shown());
    assert_eq!(server.switcher_tiles().len(), 2);
}

fn bounds_of(
    renderer: &mut Renderer,
    server: &mut Compositor,
    color: [u8; 3],
) -> (usize, usize, usize, usize) {
    renderer.render(server).expect("frame");
    let snapshot = renderer.snapshot(server).expect("read back");
    let width = snapshot.size.w as usize;
    snapshot
        .rgba
        .chunks(4)
        .enumerate()
        .filter(|(_, pixel)| pixel[..3] == color)
        .map(|(index, _)| (index % width, index / width))
        .fold(
            (usize::MAX, usize::MAX, 0, 0),
            |(left, top, right, bottom), (x, y)| {
                (left.min(x), top.min(y), right.max(x), bottom.max(y))
            },
        )
}

#[test]
fn the_overview_shows_a_dialog_on_its_parents_thumbnail() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = app_with_dialog(&mut server);
    let mut other = other_app(&mut server);
    roundtrip(&mut server, &mut [&mut editor, &mut other]);
    let end = settle(&mut server, 0);

    for key in [KEY_LEFTMETA, KEY_W] {
        server.key(key, true, 0);
    }
    for key in [KEY_W, KEY_LEFTMETA] {
        server.key(key, false, 0);
    }
    settle(&mut server, end);

    let parent = bounds_of(&mut renderer, &mut server, [0xff, 0, 0]);
    let dialog = bounds_of(&mut renderer, &mut server, [0, 0xff, 0]);
    assert!(parent.0 < dialog.0 && dialog.2 < parent.2);
    assert!(parent.1 < dialog.1 && dialog.3 < parent.3);
    assert!((parent.0 + parent.2).abs_diff(dialog.0 + dialog.2) <= 2);
}
