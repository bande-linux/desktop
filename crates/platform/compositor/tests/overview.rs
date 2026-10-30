#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use smithay::utils::Size;
use unbound_compositor::{BACKGROUND, Gesture, Headless, Renderer, Snapshot};
use unbound_test_wayland::{BLUE, RED, ShmImage, TestClient, roundtrip};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (256, 192);
const WINDOW: (i32, i32) = (64, 48);
const KEY_ESC: u32 = 1;
const KEY_LEFTALT: u32 = 56;
const KEY_LEFTMETA: u32 = 125;
const KEY_UP: u32 = 103;
const KEY_A: u32 = 30;
const BTN_LEFT: u32 = 0x110;
const ADD_WORKSPACE: (f64, f64) = (151.0, 25.0);
const BELOW_THE_STRIP: usize = 50;
const EDITOR: &str = "org.example.Editor";
const VIEWER: &str = "org.example.Viewer";

fn server() -> Headless {
    Headless::new(Size::from(OUTPUT)).expect("headless compositor")
}

fn app(server: &mut Headless, color: [u8; 4], app_id: &str) -> TestClient {
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
    client.set_app_id(server, 0, app_id);
    client
}

fn settle(server: &mut Headless, from: u64) -> u64 {
    let mut at = Duration::from_millis(from);
    while {
        at += Duration::from_millis(16);
        server.animate(at);
        server.is_animating()
    } {}
    at.as_millis() as u64
}

fn open_with_keys(server: &mut Headless) {
    server.key(KEY_LEFTALT, true, 0);
    server.key(KEY_LEFTMETA, true, 0);
    server.key(KEY_UP, true, 0);
    server.key(KEY_UP, false, 0);
    server.key(KEY_LEFTMETA, false, 0);
    server.key(KEY_LEFTALT, false, 0);
}

fn snapshot(renderer: &mut Renderer, server: &mut Headless) -> Snapshot {
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

fn at(snapshot: &Snapshot, x: i32, y: i32) -> [u8; 3] {
    let index = ((y * snapshot.size.w + x) * 4) as usize;
    [
        snapshot.rgba[index],
        snapshot.rgba[index + 1],
        snapshot.rgba[index + 2],
    ]
}

fn click(server: &mut Headless, (x, y): (f64, f64), time: u32) {
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
    let swipe = |server: &mut Headless, dy: f64, start: u32| {
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
    let below = at(&snapshot(&mut renderer, &mut server), 128, 162);

    open_with_keys(&mut server);
    settle(&mut server, end);
    let titled = at(&snapshot(&mut renderer, &mut server), 128, 162);

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
    let [old_red, _, _] = at(&halfway, 128, 80);
    let [new_red, _, _] = at(&halfway, 128, 128);

    assert!(old_red > 0x40 && old_red < 0xf0, "{old_red}");
    assert!(new_red > 0x40 && new_red < 0xf0, "{new_red}");
}
