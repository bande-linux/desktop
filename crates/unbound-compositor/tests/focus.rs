use smithay::utils::Size;
use unbound_compositor::Headless;
use unbound_test_wayland::{ShmImage, TestClient, quadrants, roundtrip};
use wayland_client::protocol::wl_shm::Format;

fn server() -> Headless {
    Headless::new(Size::from((256, 192))).expect("headless compositor")
}

fn app(server: &mut Headless) -> TestClient {
    let mut client = TestClient::connect(server);
    client.join_seat(server);
    client
}

fn open_window(server: &mut Headless, app: &mut TestClient, side: usize) {
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

fn stacking_by_side(server: &Headless) -> Vec<i32> {
    server
        .windows()
        .iter()
        .map(|window| window.frame().expect("mapped").size.w)
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
    roundtrip(&mut server, &mut [&mut other]);

    assert_eq!(other.focused_window(), Some(0));
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
