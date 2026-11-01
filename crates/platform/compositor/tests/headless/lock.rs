use std::time::{Duration, Instant};

use crate::support::*;
use bande_compositor::{BACKGROUND, Compositor, Renderer, Vblank};
use bande_test_wayland::{BLUE, ConstraintAdjustment, PopupRequest, RED, TestClient, roundtrip};
use rustix::event::{PollFd, PollFlags, Timespec, poll};

const WINDOW: (i32, i32) = (64, 48);
const INSIDE_WINDOW: (i32, i32) = (100, 80);

fn locker(server: &mut Compositor) -> TestClient {
    let mut client = TestClient::connect_as_shell(server);
    client.use_seat(server);
    client
}

fn present_frame(server: &mut Compositor, sequence: u64) {
    server.take_redraw_on(&server.main_output());
    server.latch_on(&server.main_output());
    server
        .present_on(
            &server.main_output(),
            Vblank {
                sequence,
                time: Duration::from_millis(8 * sequence),
            },
        )
        .expect("presented");
}

fn focused_app() -> (Compositor, TestClient) {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);
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
    assert_eq!(
        pixel(&mut renderer, &mut server, INSIDE_WINDOW.0, INSIDE_WINDOW.1),
        BACKGROUND
    );

    let size = lock.show_lock_surface(&mut server, BLUE);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);
    assert_eq!(size, (OUTPUT.0 as u32, OUTPUT.1 as u32));
    assert_eq!(
        pixel(&mut renderer, &mut server, INSIDE_WINDOW.0, INSIDE_WINDOW.1),
        rgb(BLUE)
    );
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
fn a_grabbing_popup_cannot_take_the_keyboard_from_the_lock_surface() {
    let (mut server, mut editor) = focused_app();
    let mut lock = locker(&mut server);
    lock.lock_session(&mut server);
    lock.show_lock_surface(&mut server, BLUE);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);

    let popup = editor.open_popup(
        &mut server,
        &PopupRequest {
            parent: 0,
            anchor_rect: (0, 0, 1, 1),
            size: (20, 20),
            adjustment: ConstraintAdjustment::empty(),
            grab: true,
            color: RED,
        },
    );
    server.key(KEY_A, true, 1);
    server.key(KEY_A, false, 2);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);

    assert!(editor.popup_dismissed(popup));
    assert!(editor.keys().is_empty());
    assert!(lock.lock_surface_has_keyboard());
    assert!(lock.keys().contains(&(KEY_A, true)));
}

#[test]
fn an_input_method_keyboard_grab_does_not_see_keys_typed_into_the_lock_screen() {
    let (mut server, mut editor) = focused_app();
    let mut input_method = TestClient::connect(&mut server);
    input_method.use_seat(&mut server);
    input_method.become_input_method(&mut server);
    input_method.grab_keyboard(&mut server);
    let mut lock = locker(&mut server);
    lock.lock_session(&mut server);
    lock.show_lock_surface(&mut server, BLUE);
    roundtrip(&mut server, &mut [&mut lock, &mut editor, &mut input_method]);

    server.key(KEY_A, true, 1);
    server.key(KEY_A, false, 2);
    roundtrip(&mut server, &mut [&mut lock, &mut editor, &mut input_method]);

    assert!(input_method.grabbed_keys().is_empty());
    assert!(lock.keys().contains(&(KEY_A, true)));
}

#[test]
fn super_shortcuts_do_nothing_while_locked() {
    let (mut server, mut editor) = focused_app();
    let mut lock = locker(&mut server);
    lock.lock_session(&mut server);
    lock.show_lock_surface(&mut server, BLUE);

    server.key(KEY_LEFTMETA, true, 1);
    server.key(KEY_H, true, 2);
    server.key(KEY_H, false, 3);
    server.key(KEY_LEFTMETA, false, 4);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);

    assert!(server.windows().iter().all(|window| !window.is_minimized()));
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
    assert_eq!(
        pixel(&mut renderer, &mut server, INSIDE_WINDOW.0, INSIDE_WINDOW.1),
        rgb(RED)
    );
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

    assert_eq!(
        pixel(&mut renderer, &mut server, INSIDE_WINDOW.0, INSIDE_WINDOW.1),
        BACKGROUND
    );
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
    assert_eq!(
        pixel(&mut renderer, &mut server, INSIDE_WINDOW.0, INSIDE_WINDOW.1),
        rgb(BLUE)
    );

    second.unlock_session(&mut server);
    roundtrip(&mut server, &mut [&mut second, &mut editor]);
    assert!(editor.keyboard_focus_on(0));
}

#[test]
fn super_l_starts_the_lock_screen_and_super_q_no_longer_quits_the_app() {
    let (mut server, mut editor) = focused_app();
    server.set_lock_screen(std::path::Path::new("/bin/true"));

    const KEY_L: u32 = 38;
    for key in [KEY_LEFTMETA, KEY_Q, KEY_L] {
        server.key(key, true, 1);
    }
    for key in [KEY_L, KEY_Q, KEY_LEFTMETA] {
        server.key(key, false, 2);
    }
    roundtrip(&mut server, &mut [&mut editor]);
    let upkeep = server.keep_session_clients_running();

    assert!(upkeep.failures.is_empty());
    assert!(server.lock_screen_pid().is_some());
    assert!(!editor.close_requested(0));
    assert!(editor.keys().contains(&(KEY_Q, true)));
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
fn logind_hears_the_session_is_locked_once_the_lock_screen_is_presented_and_unlocked_after() {
    let (mut server, mut editor) = focused_app();
    let (states, reported) = std::sync::mpsc::channel();
    server.report_lock_states(states);
    let mut lock = locker(&mut server);

    lock.lock_session(&mut server);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);
    present_frame(&mut server, 1);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);
    assert!(lock.session_locked());
    assert!(reported.try_recv().is_err());

    lock.show_lock_surface(&mut server, BLUE);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);
    assert!(reported.try_recv().is_err());
    present_frame(&mut server, 2);
    assert_eq!(reported.try_recv(), Ok(true));
    present_frame(&mut server, 3);
    assert!(reported.try_recv().is_err());

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
