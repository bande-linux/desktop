use std::time::Duration;

use smithay::utils::Size;
use unbound_compositor::{BACKGROUND, Headless, Renderer, Vblank};
use unbound_test_wayland::{BLUE, CaptureOutcome, RED, ShmImage, TestClient, roundtrip};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (256, 192);
const WINDOW: (i32, i32) = (64, 48);
const INSIDE_WINDOW: (i32, i32) = (100, 80);
const CORNER: (i32, i32) = (2, 2);
const BUFFER_CONSTRAINTS: u32 = 1;

fn server() -> Headless {
    Headless::new(Size::from(OUTPUT)).expect("headless compositor")
}

fn red_window(server: &mut Headless) -> TestClient {
    let mut client = TestClient::connect(server);
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

fn rgb_at(argb8888: &[u8], (x, y): (i32, i32)) -> [u8; 3] {
    let at = ((y * OUTPUT.0 + x) * 4) as usize;
    [argb8888[at + 2], argb8888[at + 1], argb8888[at]]
}

fn capture(server: &mut Headless, renderer: &mut Renderer, shell: &mut TestClient) -> Vec<u8> {
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
    server.take_redraw();
    server.latch();
    server
        .present(Vblank {
            sequence: 1,
            time: Duration::from_millis(8),
        })
        .expect("presented");
    shell.show_lock_surface(&mut server, BLUE);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);

    let pixels = capture(&mut server, &mut renderer, &mut shell);

    assert_eq!(rgb_at(&pixels, INSIDE_WINDOW), [0x00, 0x00, 0xff]);
    assert_eq!(rgb_at(&pixels, CORNER), [0x00, 0x00, 0xff]);
}
