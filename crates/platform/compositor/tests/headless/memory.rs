use std::path::Path;

use crate::support::*;
use smithay::utils::{Point, Size};
use unbound_compositor::{Compositor, Rect};
use unbound_test_wayland::{RED, TestClient, roundtrip};

const OUTPUT: (i32, i32) = (640, 400);
const WINDOW: (i32, i32) = (120, 80);

fn remembering(memory: &Path) -> Compositor {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    server.remember_windows_in(memory.join("unbound/windows"));
    server
}

fn open(server: &mut Compositor, app_id: &str) -> TestClient {
    let mut client = TestClient::connect(server);
    client.use_seat(server);
    client.show_window_of(server, &xrgb(&solid(RED, WINDOW), WINDOW), app_id);
    roundtrip(server, &mut [&mut client]);
    client
}

fn frame_of(server: &Compositor, app_id: &str, nth: usize) -> Rect {
    server
        .windows()
        .iter()
        .filter(|window| window.app_id() == app_id)
        .nth(nth)
        .and_then(|window| window.geometry())
        .expect("a mapped window")
}

fn drag(
    server: &mut Compositor,
    client: &mut TestClient,
    from: Point<i32, smithay::utils::Logical>,
) {
    server.move_pointer_to(at(from.x + 10, from.y + 10), 0);
    server.press_button(BTN_LEFT, true, 0);
    roundtrip(server, &mut [&mut *client]);
    client.request_move(server, 0);
    server.move_pointer_to(at(from.x + 10 + 150, from.y + 10 + 90), 1);
    server.press_button(BTN_LEFT, false, 2);
    roundtrip(server, &mut [client]);
}

#[test]
fn an_apps_first_window_opens_where_its_last_window_closed_even_after_a_restart() {
    let state = tempfile::tempdir().expect("state directory");
    let mut server = remembering(state.path());
    let mut editor = open(&mut server, EDITOR);
    let opened = frame_of(&server, EDITOR, 0);
    drag(&mut server, &mut editor, opened.location);
    let moved = frame_of(&server, EDITOR, 0);
    editor.close_window(&mut server, 0);
    drop(server);

    let mut server = remembering(state.path());
    let mut again = open(&mut server, EDITOR);
    let recalled = frame_of(&server, EDITOR, 0);
    let mut second = open(&mut server, EDITOR);
    let cascaded = frame_of(&server, EDITOR, 1);
    let viewer = open(&mut server, VIEWER);
    roundtrip(&mut server, &mut [&mut again, &mut second]);

    assert_ne!(moved.location, opened.location);
    assert_eq!(recalled.location, moved.location);
    let size = again.configured(0).expect("configured");
    assert_eq!((size.width, size.height), WINDOW);
    assert_ne!(cascaded.location, moved.location);
    assert_eq!(frame_of(&server, VIEWER, 0).location, opened.location);
    drop(viewer);
}

#[test]
fn a_remembered_frame_off_the_current_displays_is_brought_back_into_view() {
    let state = tempfile::tempdir().expect("state directory");
    let path = state.path().join("unbound/windows");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("directory");
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let displays = server.main_output();
    std::fs::write(&path, format!("{displays}\t{EDITOR}\t600\t350\t200\t150\n")).expect("memory");
    server.remember_windows_in(path);

    let _editor = open(&mut server, EDITOR);
    let frame = frame_of(&server, EDITOR, 0);

    assert!(frame.location.x + WINDOW.0 <= OUTPUT.0);
    assert!(frame.location.y + WINDOW.1 <= OUTPUT.1);
}
