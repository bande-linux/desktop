use std::time::Duration;

use crate::support::*;
use unbound_compositor::{BACKGROUND, Compositor, Renderer, Vblank};
use unbound_test_wayland::{BLUE, CaptureOutcome, RED, ShmImage, TestClient, roundtrip};
use wayland_client::protocol::wl_shm::Format;

const WINDOW: (i32, i32) = (64, 48);
const INSIDE_WINDOW: (i32, i32) = (100, 80);
const CORNER: (i32, i32) = (2, 2);
const BUFFER_CONSTRAINTS: u32 = 1;
const STOPPED: u32 = 2;
const KEY_5: u32 = 6;
const KEY_SYSRQ: u32 = 99;

fn red_window(server: &mut Compositor) -> TestClient {
    let mut client = TestClient::connect(server);
    client.show_window(server, &xrgb(&solid(RED, WINDOW), WINDOW));
    client
}

fn rgb_at(argb8888: &[u8], (x, y): (i32, i32)) -> [u8; 3] {
    let at = ((y * OUTPUT.0 + x) * 4) as usize;
    [argb8888[at + 2], argb8888[at + 1], argb8888[at]]
}

fn capture(server: &mut Compositor, renderer: &mut Renderer, shell: &mut TestClient) -> Vec<u8> {
    shell.start_screen_capture(server, false);
    shell.capture_frame(server, OUTPUT);
    renderer.render(server).expect("frame");
    shell.exchange(server);
    match shell.capture_outcome() {
        Some(CaptureOutcome::Ready(pixels)) => pixels.clone(),
        other => panic!("capture did not complete: {other:?}"),
    }
}

#[test]
fn only_the_shell_may_capture_the_screen() {
    let mut server = server();
    let app = TestClient::connect(&mut server);
    let shell = TestClient::connect_as_shell(&mut server);

    assert!(!app.can_capture_screen());
    assert!(shell.can_capture_screen());
}

#[test]
fn an_output_session_asks_for_an_output_sized_buffer_in_the_shm_formats() {
    let mut server = server();
    let mut shell = TestClient::connect_as_shell(&mut server);

    let constraints = shell.start_screen_capture(&mut server, false);

    assert_eq!(constraints.size, Some((256, 192)));
    assert!(constraints.shm_formats.contains(&Format::Argb8888));
    assert!(constraints.shm_formats.contains(&Format::Xrgb8888));
    assert!(constraints.done);
    assert!(!constraints.stopped);
}

#[test]
fn a_captured_frame_holds_what_the_screen_shows() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut app = red_window(&mut server);
    let mut shell = TestClient::connect_as_shell(&mut server);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);

    let pixels = capture(&mut server, &mut renderer, &mut shell);

    assert_eq!(pixels.len(), (OUTPUT.0 * OUTPUT.1 * 4) as usize);
    assert_eq!(rgb_at(&pixels, INSIDE_WINDOW), [0xff, 0x00, 0x00]);
    assert_eq!(rgb_at(&pixels, CORNER), BACKGROUND);
    let screen = renderer.snapshot(&server).expect("read back");
    let screen_as_argb: Vec<u8> = screen
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[red, green, blue, alpha]| [blue, green, red, alpha])
        .collect();
    assert_eq!(pixels, screen_as_argb);
}

#[test]
fn a_buffer_of_the_wrong_size_fails_on_its_constraints() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.start_screen_capture(&mut server, false);

    shell.capture_frame(&mut server, (OUTPUT.0 / 2, OUTPUT.1));
    renderer.render(&mut server).expect("frame");
    shell.exchange(&mut server);

    assert_eq!(
        shell.capture_outcome(),
        Some(&CaptureOutcome::Failed(BUFFER_CONSTRAINTS))
    );
}

#[test]
fn a_locked_screen_captures_only_the_lock_surface() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut app = red_window(&mut server);
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.lock_session(&mut server);
    server.take_redraw_on(&server.main_output());
    server.latch_on(&server.main_output());
    server
        .present_on(
            &server.main_output(),
            Vblank {
                sequence: 1,
                time: Duration::from_millis(8),
            },
        )
        .expect("presented");
    shell.show_lock_surface(&mut server, BLUE);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);

    let pixels = capture(&mut server, &mut renderer, &mut shell);

    assert_eq!(rgb_at(&pixels, INSIDE_WINDOW), [0x00, 0x00, 0xff]);
    assert_eq!(rgb_at(&pixels, CORNER), [0x00, 0x00, 0xff]);
}

fn red_window_named(server: &mut Compositor, app_id: &str) -> TestClient {
    let mut client = red_window(server);
    client.set_app_id(server, 0, app_id);
    client
}

fn capture_window(
    server: &mut Compositor,
    renderer: &mut Renderer,
    shell: &mut TestClient,
    size: (i32, i32),
) -> Vec<u8> {
    shell.capture_frame(server, size);
    renderer.render(server).expect("frame");
    shell.exchange(server);
    match shell.capture_outcome() {
        Some(CaptureOutcome::Ready(pixels)) => pixels.clone(),
        other => panic!("capture did not complete: {other:?}"),
    }
}

fn argb_at(argb8888: &[u8], width: i32, (x, y): (i32, i32)) -> [u8; 4] {
    let at = ((y * width + x) * 4) as usize;
    [
        argb8888[at + 3],
        argb8888[at + 2],
        argb8888[at + 1],
        argb8888[at],
    ]
}

fn named_shell(server: &mut Compositor, app: &mut TestClient) -> TestClient {
    let mut shell = TestClient::connect_as_shell(server);
    shell.join_shell(server);
    roundtrip(server, &mut [app, &mut shell]);
    shell
}

#[test]
fn a_window_capture_holds_only_the_window_with_clear_rounded_corners() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut blue = TestClient::connect(&mut server);
    blue.show_window(
        &mut server,
        &ShmImage {
            width: OUTPUT.0,
            height: OUTPUT.1,
            stride: OUTPUT.0 * 4,
            format: Format::Xrgb8888,
            bytes: &BLUE.repeat((OUTPUT.0 * OUTPUT.1) as usize),
        },
    );
    let mut app = red_window_named(&mut server, "org.example.Red");
    let mut shell = named_shell(&mut server, &mut app);

    let constraints = shell.start_window_capture(&mut server, "org.example.Red", false);
    let pixels = capture_window(&mut server, &mut renderer, &mut shell, WINDOW);

    assert_eq!(constraints.size, Some((WINDOW.0 as u32, WINDOW.1 as u32)));
    assert_eq!(
        argb_at(&pixels, WINDOW.0, (WINDOW.0 / 2, WINDOW.1 / 2)),
        [0xff, 0xff, 0x00, 0x00]
    );
    assert_eq!(argb_at(&pixels, WINDOW.0, (0, 0))[0], 0);
}

#[test]
fn the_shell_learns_where_the_windows_are_frontmost_first() {
    let mut server = server();
    let mut back = red_window_named(&mut server, "org.example.Back");
    let mut front = red_window_named(&mut server, "org.example.Front");
    let mut shell = named_shell(&mut server, &mut back);
    shell.join_capture(&mut server);
    roundtrip(&mut server, &mut [&mut back, &mut front, &mut shell]);
    let geometry = |index: usize| {
        let window = &server.windows()[index];
        let geometry = window.geometry().expect("mapped");
        (
            geometry.location.x,
            geometry.location.y,
            geometry.size.w,
            geometry.size.h,
        )
    };
    let expected = vec![
        (
            shell.window_identifiers("org.example.Front")[0].clone(),
            geometry(1),
        ),
        (
            shell.window_identifiers("org.example.Back")[0].clone(),
            geometry(0),
        ),
    ];

    let windows = shell.list_windows(&mut server);

    assert_eq!(windows, expected);
    assert!(!TestClient::connect(&mut server).advertises("unbound_capture_v1"));
}

#[test]
fn super_shift_5_asks_the_shell_for_the_capture_options() {
    let mut server = server();
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_capture(&mut server);
    let keys = [KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_5];

    for key in keys {
        server.key(key, true, 0);
    }
    for key in keys.iter().rev() {
        server.key(*key, false, 0);
    }
    shell.exchange(&mut server);

    assert_eq!(shell.capture_option_requests(), 1);
}

#[test]
fn the_print_screen_key_asks_the_shell_for_the_capture_options() {
    let mut server = server();
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_capture(&mut server);

    tap(&mut server, KEY_SYSRQ);
    shell.exchange(&mut server);

    assert_eq!(shell.capture_option_requests(), 1);
}

#[test]
fn a_stream_frame_waits_for_something_to_change_after_the_first() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut app = red_window(&mut server);
    let mut shell = TestClient::connect_as_shell(&mut server);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);
    shell.start_screen_capture(&mut server, false);
    server.take_redraw_on(&server.main_output());

    shell.capture_frame(&mut server, OUTPUT);
    let first_redraws = server.take_redraw_on(&server.main_output());
    renderer.render(&mut server).expect("frame");
    shell.exchange(&mut server);
    shell.capture_frame(&mut server, OUTPUT);
    let second_redraws = server.take_redraw_on(&server.main_output());

    assert!(first_redraws);
    assert!(!second_redraws);
}

#[test]
fn a_resized_window_announces_its_new_size_before_the_next_frame() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut app = red_window_named(&mut server, "org.example.Red");
    let mut shell = named_shell(&mut server, &mut app);
    shell.start_window_capture(&mut server, "org.example.Red", false);
    capture_window(&mut server, &mut renderer, &mut shell, WINDOW);
    let wider = (WINDOW.0 * 2, WINDOW.1);
    app.commit_buffer(
        0,
        &ShmImage {
            width: wider.0,
            height: wider.1,
            stride: wider.0 * 4,
            format: Format::Xrgb8888,
            bytes: &RED.repeat((wider.0 * wider.1) as usize),
        },
        (0, 0, wider.0, wider.1),
    );
    roundtrip(&mut server, &mut [&mut app, &mut shell]);

    shell.capture_frame(&mut server, WINDOW);
    renderer.render(&mut server).expect("frame");
    shell.exchange(&mut server);

    assert_eq!(
        shell.capture_outcome(),
        Some(&CaptureOutcome::Failed(BUFFER_CONSTRAINTS))
    );
    assert_eq!(
        shell.capture_constraints().size,
        Some((wider.0 as u32, wider.1 as u32))
    );
    let pixels = capture_window(&mut server, &mut renderer, &mut shell, wider);
    assert_eq!(pixels.len(), (wider.0 * wider.1 * 4) as usize);
}

#[test]
fn a_closed_window_stops_its_capture() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut app = red_window_named(&mut server, "org.example.Red");
    let mut shell = named_shell(&mut server, &mut app);
    shell.start_window_capture(&mut server, "org.example.Red", false);
    capture_window(&mut server, &mut renderer, &mut shell, WINDOW);
    shell.capture_frame(&mut server, WINDOW);

    app.close_window(&mut server, 0);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);
    renderer.render(&mut server).expect("frame");
    shell.exchange(&mut server);

    assert_eq!(
        shell.capture_outcome(),
        Some(&CaptureOutcome::Failed(STOPPED))
    );
    assert!(shell.capture_constraints().stopped);
}
