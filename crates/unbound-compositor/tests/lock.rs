use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use smithay::utils::Size;
use unbound_compositor::{BACKGROUND, Headless, Renderer, Vblank};
use unbound_test_wayland::{BLUE, RED, ShmImage, TestClient, roundtrip};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (256, 192);
const WINDOW: (i32, i32) = (64, 48);
const INSIDE_WINDOW: (i32, i32) = (100, 80);
const BTN_LEFT: u32 = 0x110;
const KEY_A: u32 = 30;
const KEY_Q: u32 = 16;
const KEY_LEFTMETA: u32 = 125;

fn server() -> Headless {
    Headless::new(Size::from(OUTPUT)).expect("headless compositor")
}

fn app(server: &mut Headless) -> TestClient {
    let mut client = TestClient::connect(server);
    client.use_seat(server);
    let pixels = RED.repeat((WINDOW.0 * WINDOW.1) as usize);
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
    client
}

fn locker(server: &mut Headless) -> TestClient {
    let mut client = TestClient::connect_as_shell(server);
    client.use_seat(server);
    client
}

fn present_frame(server: &mut Headless, sequence: u64) {
    server.take_redraw();
    server.latch();
    server
        .present(Vblank {
            sequence,
            time: Duration::from_millis(8 * sequence),
        })
        .expect("presented");
}

fn pixel(renderer: &mut Renderer, server: &mut Headless, (x, y): (i32, i32)) -> [u8; 3] {
    renderer.render(server).expect("frame");
    let snapshot = renderer.snapshot(server).expect("read back");
    let at = ((y * snapshot.size.w + x) * 4) as usize;
    [
        snapshot.rgba[at],
        snapshot.rgba[at + 1],
        snapshot.rgba[at + 2],
    ]
}

fn rgb(bgra: [u8; 4]) -> [u8; 3] {
    [bgra[2], bgra[1], bgra[0]]
}

fn focused_app() -> (Headless, TestClient) {
    let mut server = server();
    let mut editor = app(&mut server);
    server.move_pointer_to((128.0, 96.0).into(), 0);
    server.press_button(BTN_LEFT, true, 0);
    server.press_button(BTN_LEFT, false, 0);
    roundtrip(&mut server, &mut [&mut editor]);
    assert!(editor.keyboard_focus_on(0));
    (server, editor)
}

#[test]
fn only_the_shell_may_lock_the_session() {
    let mut server = server();
    let editor = TestClient::connect(&mut server);
    let shell = TestClient::connect_as_shell(&mut server);

    assert!(!editor.can_lock_session());
    assert!(shell.can_lock_session());
}

#[test]
fn the_session_is_reported_locked_only_after_a_blank_frame_is_presented() {
    let (mut server, mut editor) = focused_app();
    let mut lock = locker(&mut server);

    lock.lock_session(&mut server);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);
    assert!(!lock.session_locked());
    assert!(!editor.keyboard_focus_on(0));

    present_frame(&mut server, 1);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);
    assert!(lock.session_locked());
}

#[test]
fn while_locked_only_the_lock_surface_is_drawn_and_receives_input() {
    let (mut server, mut editor) = focused_app();
    let mut renderer = Renderer::new().expect("renderer");
    let mut lock = locker(&mut server);
    lock.lock_session(&mut server);
    present_frame(&mut server, 1);
    assert_eq!(pixel(&mut renderer, &mut server, INSIDE_WINDOW), BACKGROUND);

    let size = lock.show_lock_surface(&mut server, BLUE);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);
    assert_eq!(size, (OUTPUT.0 as u32, OUTPUT.1 as u32));
    assert_eq!(pixel(&mut renderer, &mut server, INSIDE_WINDOW), rgb(BLUE));
    assert!(lock.lock_surface_has_keyboard());
    let clicks_before = editor.buttons().len();

    server.key(KEY_A, true, 1);
    server.key(KEY_A, false, 2);
    server.move_pointer_to((128.0, 96.0).into(), 3);
    server.press_button(BTN_LEFT, true, 4);
    server.press_button(BTN_LEFT, false, 5);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);
    assert!(editor.keys().is_empty());
    assert_eq!(editor.buttons().len(), clicks_before);
    assert!(lock.keys().contains(&(KEY_A, true)));
    assert!(!editor.keyboard_focus_on(0));
}

#[test]
fn super_shortcuts_do_nothing_while_locked() {
    let (mut server, mut editor) = focused_app();
    let mut lock = locker(&mut server);
    lock.lock_session(&mut server);
    lock.show_lock_surface(&mut server, BLUE);

    server.key(KEY_LEFTMETA, true, 1);
    server.key(KEY_Q, true, 2);
    server.key(KEY_Q, false, 3);
    server.key(KEY_LEFTMETA, false, 4);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);

    assert!(!editor.close_requested(0));
}

#[test]
fn unlocking_returns_the_keyboard_to_the_window_that_had_it() {
    let (mut server, mut editor) = focused_app();
    let mut renderer = Renderer::new().expect("renderer");
    let mut lock = locker(&mut server);
    lock.lock_session(&mut server);
    present_frame(&mut server, 1);
    lock.show_lock_surface(&mut server, BLUE);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);

    lock.unlock_session(&mut server);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);

    assert!(editor.keyboard_focus_on(0));
    assert_eq!(pixel(&mut renderer, &mut server, INSIDE_WINDOW), rgb(RED));
}

#[test]
fn the_session_stays_locked_and_blank_when_the_lock_client_dies() {
    let (mut server, mut editor) = focused_app();
    let mut renderer = Renderer::new().expect("renderer");
    let mut lock = locker(&mut server);
    lock.lock_session(&mut server);
    present_frame(&mut server, 1);
    lock.show_lock_surface(&mut server, BLUE);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);

    drop(lock);
    roundtrip(&mut server, &mut [&mut editor]);
    server.key(KEY_A, true, 1);
    roundtrip(&mut server, &mut [&mut editor]);

    assert_eq!(pixel(&mut renderer, &mut server, INSIDE_WINDOW), BACKGROUND);
    assert!(editor.keys().is_empty());
}

#[test]
fn a_second_lock_is_refused_while_the_first_locker_is_alive() {
    let (mut server, mut editor) = focused_app();
    let mut first = locker(&mut server);
    first.lock_session(&mut server);
    present_frame(&mut server, 1);
    let mut second = locker(&mut server);

    second.lock_session(&mut server);
    roundtrip(&mut server, &mut [&mut first, &mut second, &mut editor]);

    assert!(second.lock_refused());
    assert!(first.session_locked());
}

#[test]
fn a_new_locker_takes_over_after_the_first_one_dies() {
    let (mut server, mut editor) = focused_app();
    let mut renderer = Renderer::new().expect("renderer");
    let mut first = locker(&mut server);
    first.lock_session(&mut server);
    present_frame(&mut server, 1);
    drop(first);
    roundtrip(&mut server, &mut [&mut editor]);

    let mut second = locker(&mut server);
    second.lock_session(&mut server);
    present_frame(&mut server, 2);
    roundtrip(&mut server, &mut [&mut second, &mut editor]);
    assert!(second.session_locked());
    second.show_lock_surface(&mut server, BLUE);
    assert_eq!(pixel(&mut renderer, &mut server, INSIDE_WINDOW), rgb(BLUE));

    second.unlock_session(&mut server);
    roundtrip(&mut server, &mut [&mut second, &mut editor]);
    assert!(editor.keyboard_focus_on(0));
}

const KEY_LEFTCTRL: u32 = 29;

#[test]
fn ctrl_super_q_starts_the_lock_screen_without_quitting_the_app() {
    let (mut server, mut editor) = focused_app();
    server.set_lock_screen(std::path::Path::new("/bin/true"));

    for key in [KEY_LEFTCTRL, KEY_LEFTMETA, KEY_Q] {
        server.key(key, true, 1);
    }
    for key in [KEY_Q, KEY_LEFTMETA, KEY_LEFTCTRL] {
        server.key(key, false, 2);
    }
    roundtrip(&mut server, &mut [&mut editor]);
    let upkeep = server.keep_session_clients_running();

    assert!(upkeep.failures.is_empty());
    assert!(server.lock_screen_pid().is_some());
    assert!(!editor.close_requested(0));
    assert!(!editor.keys().iter().any(|(key, _)| *key == KEY_Q));
}

#[test]
fn the_lock_screen_is_started_again_when_the_locker_dies_while_locked() {
    let (mut server, mut editor) = focused_app();
    server.set_lock_screen(std::path::Path::new("/bin/true"));
    let mut lock = locker(&mut server);
    lock.lock_session(&mut server);
    present_frame(&mut server, 1);
    server.keep_session_clients_running();
    assert!(server.lock_screen_pid().is_none());

    drop(lock);
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.lock_screen_pid().is_none() {
        assert!(
            Instant::now() < deadline,
            "the compositor never started a lock screen after the locker died"
        );
        roundtrip(&mut server, &mut [&mut editor]);
        server.keep_session_clients_running();
    }
}

#[test]
fn logind_hears_the_session_is_locked_once_blank_and_unlocked_after() {
    let (mut server, mut editor) = focused_app();
    let (states, reported) = std::sync::mpsc::channel();
    server.report_lock_states(states);
    let mut lock = locker(&mut server);

    lock.lock_session(&mut server);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);
    assert!(reported.try_recv().is_err());
    present_frame(&mut server, 1);
    assert_eq!(reported.try_recv(), Ok(true));

    lock.show_lock_surface(&mut server, BLUE);
    lock.unlock_session(&mut server);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);
    assert_eq!(reported.try_recv(), Ok(false));
}

#[test]
fn a_lock_request_from_logind_starts_the_lock_screen() {
    let (mut server, _editor) = focused_app();
    server.set_lock_screen(std::path::Path::new("/bin/true"));

    server.request_lock_screen();
    server.keep_session_clients_running();

    assert!(server.lock_screen_pid().is_some());
}

#[test]
fn a_lock_screen_that_exits_after_disconnecting_is_reaped_without_other_events() {
    let script = std::env::temp_dir().join(format!("disconnect-then-exit-{}", std::process::id()));
    std::fs::write(
        &script,
        "#!/bin/sh\neval \"exec $WAYLAND_SOCKET>&-\"\nsleep 0.5\n",
    )
    .expect("write the lock screen stand-in");
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("make it executable");
    let mut server = server();
    server.set_lock_screen(&script);
    server.request_lock_screen();
    server.keep_session_clients_running();
    assert!(server.lock_screen_pid().is_some());

    let fds = [
        server.poll_fd().expect("display fd"),
        server.fence_fd().expect("internal loop fd"),
    ];
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.lock_screen_pid().is_some() {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let mut polled = fds
            .iter()
            .map(|fd| PollFd::new(fd, PollFlags::IN))
            .collect::<Vec<_>>();
        let woken = poll(
            &mut polled,
            Some(&Timespec::try_from(left).expect("timeout")),
        )
        .expect("poll the compositor's fds");
        assert!(
            woken > 0,
            "nothing woke the compositor to reap the lock screen"
        );
        server.dispatch().expect("dispatch");
        server.keep_session_clients_running();
    }
    let _ = std::fs::remove_file(script);
}
