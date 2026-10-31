use crate::support::*;
use unbound_compositor::Compositor;
use unbound_test_wayland::{
    SeparateApp, ShmImage, TestClient, act_as_separate_app, quadrants, roundtrip, roundtrip_until,
};
use wayland_client::protocol::wl_shm::Format;

fn app(server: &mut Compositor) -> TestClient {
    let mut client = TestClient::connect(server);
    client.join_seat(server);
    client
}

fn open_window(server: &mut Compositor, app: &mut TestClient, side: usize) {
    let stride = side * 4;
    let pixels = quadrants(side, side, stride);
    let side = i32::try_from(side).expect("side");
    app.show_window(
        server,
        &ShmImage {
            width: side,
            height: side,
            stride: side * 4,
            format: Format::Argb8888,
            bytes: &pixels,
        },
    );
}

fn stacking_by_side(server: &Compositor) -> Vec<i32> {
    server
        .windows()
        .iter()
        .map(|window| window.size().expect("mapped").w)
        .collect()
}

#[test]
fn closing_the_focused_window_focuses_the_same_apps_next_window() {
    let mut server = server();
    let mut other = app(&mut server);
    open_window(&mut server, &mut other, 2);
    let mut editor = app(&mut server);
    open_window(&mut server, &mut editor, 3);
    open_window(&mut server, &mut editor, 4);
    roundtrip(&mut server, &mut [&mut other, &mut editor]);
    assert_eq!(editor.focused_window(), Some(1));

    editor.close_window(&mut server, 1);
    roundtrip(&mut server, &mut [&mut other, &mut editor]);

    assert_eq!(editor.focused_window(), Some(0));
    assert!(!other.has_keyboard_focus());
}

#[test]
fn closing_an_apps_last_window_leaves_no_window_focused() {
    let mut server = server();
    let mut other = app(&mut server);
    open_window(&mut server, &mut other, 2);
    let mut editor = app(&mut server);
    open_window(&mut server, &mut editor, 3);
    roundtrip(&mut server, &mut [&mut other, &mut editor]);
    assert!(editor.has_keyboard_focus());

    editor.close_window(&mut server, 0);
    roundtrip(&mut server, &mut [&mut other, &mut editor]);

    assert!(!editor.has_keyboard_focus());
    assert!(!other.has_keyboard_focus());
}

#[test]
fn quitting_the_focused_app_focuses_the_frontmost_remaining_window() {
    let mut server = server();
    let mut other = app(&mut server);
    open_window(&mut server, &mut other, 2);
    let mut editor = app(&mut server);
    open_window(&mut server, &mut editor, 3);
    roundtrip(&mut server, &mut [&mut other, &mut editor]);
    assert!(!other.has_keyboard_focus());

    drop(editor);

    roundtrip_until(&mut server, &mut [&mut other], |apps| {
        apps[0].focused_window() == Some(0)
    });
}

#[test]
fn an_app_that_closes_its_last_window_and_then_quits_hands_focus_to_the_next_app() {
    let mut server = server();
    let mut other = app(&mut server);
    open_window(&mut server, &mut other, 2);
    let mut editor = app(&mut server);
    open_window(&mut server, &mut editor, 3);
    roundtrip(&mut server, &mut [&mut other, &mut editor]);

    editor.close_window(&mut server, 0);
    roundtrip(&mut server, &mut [&mut other, &mut editor]);
    assert!(!other.has_keyboard_focus());
    drop(editor);

    roundtrip_until(&mut server, &mut [&mut other], |apps| {
        apps[0].focused_window() == Some(0)
    });
}

#[test]
fn a_background_apps_new_window_opens_behind_the_focused_window() {
    let mut server = server();
    let mut background = app(&mut server);
    open_window(&mut server, &mut background, 2);
    let mut foreground = app(&mut server);
    open_window(&mut server, &mut foreground, 4);

    open_window(&mut server, &mut background, 3);
    roundtrip(&mut server, &mut [&mut background, &mut foreground]);

    assert_eq!(foreground.focused_window(), Some(0));
    assert!(!background.has_keyboard_focus());
    assert_eq!(stacking_by_side(&server), vec![2, 3, 4]);
}

fn type_a_key(server: &mut Compositor) {
    server.key(KEY_A, true, 0);
    server.key(KEY_A, false, 40);
}

#[test]
fn an_app_opening_unasked_while_you_type_opens_behind_and_asks_for_attention() {
    if act_as_separate_app() {
        return;
    }
    let mut server = server();
    let mut dock = TestClient::connect_as_shell(&mut server);
    dock.use_seat(&mut server);
    dock.join_shell(&mut server);
    let mut terminal = app(&mut server);
    open_window(&mut server, &mut terminal, 2);
    roundtrip(&mut server, &mut [&mut dock, &mut terminal]);
    let mut intruder = SeparateApp::launch(
        &mut server,
        "focus::an_app_opening_unasked_while_you_type_opens_behind_and_asks_for_attention",
    );
    type_a_key(&mut server);

    intruder.show_window(&mut server, 4);
    roundtrip(&mut server, &mut [&mut dock, &mut terminal]);

    assert!(terminal.has_keyboard_focus());
    assert_eq!(stacking_by_side(&server), vec![4, 2]);
    assert_eq!(dock.attention_requests().len(), 1);
}

#[test]
fn the_app_you_type_into_brings_its_new_window_forward() {
    let mut server = server();
    let mut files = app(&mut server);
    open_window(&mut server, &mut files, 2);
    roundtrip(&mut server, &mut [&mut files]);
    type_a_key(&mut server);

    open_window(&mut server, &mut files, 4);
    roundtrip(&mut server, &mut [&mut files]);

    assert_eq!(files.focused_window(), Some(1));
    assert_eq!(stacking_by_side(&server), vec![2, 4]);
}

#[test]
fn an_app_the_shell_opened_comes_forward_even_while_you_type() {
    let mut server = server();
    let mut launcher = TestClient::connect_as_shell(&mut server);
    launcher.use_seat(&mut server);
    launcher.join_shell(&mut server);
    let mut terminal = app(&mut server);
    open_window(&mut server, &mut terminal, 2);
    roundtrip(&mut server, &mut [&mut launcher, &mut terminal]);
    type_a_key(&mut server);
    let token = launcher.activation_token(&mut server);

    let mut opened = app(&mut server);
    let pixels = quadrants(4, 4, 16);
    opened.show_window_activated(
        &mut server,
        &ShmImage {
            width: 4,
            height: 4,
            stride: 16,
            format: Format::Argb8888,
            bytes: &pixels,
        },
        &token,
    );
    roundtrip(
        &mut server,
        &mut [&mut launcher, &mut terminal, &mut opened],
    );

    assert!(opened.has_keyboard_focus());
    assert!(!terminal.has_keyboard_focus());
}
