use std::time::Duration;

use smithay::utils::Size;
use unbound_compositor::Headless;
use unbound_test_wayland::{IdleEvent, RED, ShmImage, TestClient, roundtrip};
use wayland_client::protocol::wl_shm::Format;

const KEY_A: u32 = 30;
const SECOND: Duration = Duration::from_secs(1);

fn server() -> Headless {
    Headless::new(Size::from((256, 192))).expect("headless compositor")
}

fn watcher(server: &mut Headless) -> TestClient {
    let mut client = TestClient::connect(server);
    client.use_seat(server);
    client
}

fn tick(server: &mut Headless, client: &mut TestClient, at: Duration) -> Option<Duration> {
    let next = server.idle_tick(at);
    roundtrip(server, &mut [client]);
    next
}

#[test]
fn the_session_goes_idle_after_the_timeout_without_input_and_resumes_on_input() {
    let mut server = server();
    let mut power = watcher(&mut server);
    let watch = power.watch_idle(&mut server, 5_000, false);

    assert_eq!(tick(&mut server, &mut power, SECOND), Some(6 * SECOND));
    tick(&mut server, &mut power, 5 * SECOND);
    assert!(power.idle_events(watch).is_empty());

    tick(&mut server, &mut power, 6 * SECOND);
    assert_eq!(power.idle_events(watch), [IdleEvent::Idled]);

    server.key(KEY_A, true, 0);
    server.key(KEY_A, false, 0);
    assert_eq!(tick(&mut server, &mut power, 7 * SECOND), Some(12 * SECOND));
    assert_eq!(
        power.idle_events(watch),
        [IdleEvent::Idled, IdleEvent::Resumed]
    );
}

#[test]
fn input_restarts_the_countdown() {
    let mut server = server();
    let mut power = watcher(&mut server);
    let watch = power.watch_idle(&mut server, 5_000, false);
    tick(&mut server, &mut power, SECOND);

    server.move_pointer_to((10.0, 10.0).into(), 0);
    tick(&mut server, &mut power, 4 * SECOND);
    tick(&mut server, &mut power, 8 * SECOND);
    assert!(power.idle_events(watch).is_empty());

    tick(&mut server, &mut power, 9 * SECOND);
    assert_eq!(power.idle_events(watch), [IdleEvent::Idled]);
}

#[test]
fn a_visible_window_inhibiting_idle_holds_off_only_the_inhibitable_notification() {
    let mut server = server();
    let mut power = watcher(&mut server);
    let inhibitable = power.watch_idle(&mut server, 5_000, false);
    let input_only = power.watch_idle(&mut server, 5_000, true);
    let pixels = RED.repeat(32 * 32);
    let mut player = TestClient::connect(&mut server);
    player.show_window(
        &mut server,
        &ShmImage {
            width: 32,
            height: 32,
            stride: 128,
            format: Format::Xrgb8888,
            bytes: &pixels,
        },
    );
    player.inhibit_idle(&mut server, 0);
    tick(&mut server, &mut power, SECOND);

    tick(&mut server, &mut power, 60 * SECOND);
    assert!(power.idle_events(inhibitable).is_empty());
    assert_eq!(power.idle_events(input_only), [IdleEvent::Idled]);

    player.close_window(&mut server, 0);
    tick(&mut server, &mut power, 61 * SECOND);
    assert_eq!(power.idle_events(inhibitable), [IdleEvent::Idled]);
}
