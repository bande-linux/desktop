use std::time::{Duration, Instant};

use crate::support::*;
use accesskit::Role;
use bande_compositor::{Compositor, Renderer, UNRESPONSIVE_AFTER};
use bande_test_wayland::{BLUE, RED, SeparateApp, TestClient, act_as_separate_app, roundtrip};
use smithay::utils::Size;

const SCREEN: (i32, i32) = (560, 520);
const KEY_ENTER: u32 = 28;
const STUCK: &str = "org.example.Stuck";

fn server() -> Compositor {
    let mut server = Compositor::new(Size::from(SCREEN)).expect("headless compositor");
    server.set_reduce_motion(true);
    server
}

fn force_quit_keys(server: &mut Compositor) {
    chord(server, &[KEY_LEFTCTRL, KEY_LEFTALT, KEY_ESC]);
}

fn mark_hung(server: &mut Compositor, clients: &mut [&mut TestClient]) {
    let start = Duration::from_secs(10);
    server.watchdog_tick(start);
    roundtrip(server, clients);
    server.watchdog_tick(start + UNRESPONSIVE_AFTER);
    server.force_quit_tick(Instant::now());
}

fn typed(client: &TestClient) -> Vec<u32> {
    client.keys().iter().map(|(key, _)| *key).collect()
}

#[test]
fn ctrl_alt_escape_opens_force_quit_on_the_frontmost_app_and_escape_closes_it() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, (64, 64), EDITOR);
    let mut viewer = window_of(&mut server, BLUE, (64, 64), VIEWER);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    force_quit_keys(&mut server);
    let rows = server.force_quit_rows();
    let selection = server.force_quit_selection();
    let upkeep = server.keep_session_clients_running();
    tap(&mut server, KEY_ESC);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);

    assert_eq!(rows, ["Editor", "Viewer"]);
    assert_eq!(selection, ["Viewer"]);
    assert!(upkeep.failures.is_empty());
    assert!(server.force_quit_frame().is_none());
    assert!(!typed(&viewer).contains(&KEY_ESC));
}

#[test]
fn a_hung_app_is_marked_and_force_quit_from_the_keyboard_while_the_shell_is_hung() {
    if act_as_separate_app() {
        return;
    }
    let mut server = server();
    let mut bar = TestClient::connect_as_shell(&mut server);
    bar.join_shell(&mut server);
    let mut stuck = SeparateApp::launch(
        &mut server,
        "force_quit::a_hung_app_is_marked_and_force_quit_from_the_keyboard_while_the_shell_is_hung",
    );
    stuck.show_window_of(&mut server, 64, STUCK);
    let mut editor = window_of(&mut server, RED, (64, 64), EDITOR);
    editor.inhibit_shortcuts(&mut server, 0);
    roundtrip(&mut server, &mut [&mut editor]);

    force_quit_keys(&mut server);
    mark_hung(&mut server, &mut [&mut editor]);
    let rows = server.force_quit_rows();
    let selection = server.force_quit_selection();
    tap(&mut server, KEY_DOWN);
    tap(&mut server, KEY_ENTER);
    let asked = server.force_quit_asks();
    tap(&mut server, KEY_ENTER);
    let deadline = Instant::now() + Duration::from_secs(10);
    while server
        .windows()
        .iter()
        .any(|window| window.app_id() == STUCK)
    {
        assert!(Instant::now() < deadline, "the hung app is still running");
        server.dispatch().expect("server dispatch");
        std::thread::sleep(Duration::from_millis(5));
    }
    server.force_quit_tick(Instant::now());
    roundtrip(&mut server, &mut [&mut editor]);

    assert_eq!(rows, ["Editor", "Stuck (Not Responding)"]);
    assert_eq!(selection, ["Editor"]);
    assert!(asked);
    assert!(!server.force_quit_asks());
    assert_eq!(server.force_quit_rows(), ["Editor"]);
    assert!(
        typed(&editor)
            .iter()
            .all(|key| ![KEY_ESC, KEY_DOWN, KEY_ENTER].contains(key))
    );
}

#[test]
fn force_quit_never_kills_the_process_it_runs_in() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, (64, 64), EDITOR);
    roundtrip(&mut server, &mut [&mut editor]);

    server.show_force_quit();
    tap(&mut server, KEY_ENTER);
    tap(&mut server, KEY_ENTER);
    roundtrip(&mut server, &mut [&mut editor]);
    server.force_quit_tick(Instant::now());

    assert_eq!(server.force_quit_rows(), ["Editor"]);
}

#[test]
fn the_system_menu_opens_force_quit_and_a_click_beside_it_closes_it() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, (64, 64), EDITOR);
    let mut menu_bar = TestClient::connect_as_shell(&mut server);
    menu_bar.join_shell(&mut server);

    menu_bar.show_force_quit(&mut server);
    let frame = server.force_quit_frame();
    click(&mut server, 2, SCREEN.1 - 2);
    roundtrip(&mut server, &mut [&mut editor]);

    let frame = frame.expect("Force Quit opens");
    assert!(frame.size.w > 0.0 && frame.size.h > 0.0);
    assert!(server.force_quit_frame().is_none());
    assert!(editor.buttons().is_empty());
}

#[test]
fn screen_readers_meet_a_force_quit_dialog_that_names_the_hung_app() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, (64, 64), EDITOR);
    let mut viewer = window_of(&mut server, BLUE, (64, 64), VIEWER);
    viewer.hang();

    server.show_force_quit();
    mark_hung(&mut server, &mut [&mut editor]);
    let tree = server
        .force_quit_accessibility()
        .expect("an open Force Quit");

    let root = tree.nodes.first().map(|(_, node)| node).expect("a root");
    assert_eq!(root.role(), Role::Dialog);
    assert_eq!(root.label(), Some("Force Quit Apps"));
    assert!(
        tree.nodes
            .iter()
            .any(|(_, node)| node.label() == Some("Viewer (Not Responding)"))
    );
    viewer.recover(&mut server);
}

#[test]
fn force_quit_matches_its_golden() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = window_of(&mut server, RED, (64, 64), EDITOR);
    let mut viewer = window_of(&mut server, BLUE, (64, 64), VIEWER);
    viewer.hang();

    server.show_force_quit();
    mark_hung(&mut server, &mut [&mut editor]);
    tap(&mut server, KEY_UP);
    settle(&mut server, 0);
    renderer.render(&mut server).expect("frame");
    let snapshot = renderer.snapshot(&server).expect("read back");
    viewer.recover(&mut server);

    bande_golden::assert_golden_rgba!(
        "force-quit",
        (
            snapshot.size.w.unsigned_abs(),
            snapshot.size.h.unsigned_abs()
        ),
        snapshot.rgba
    );
}
