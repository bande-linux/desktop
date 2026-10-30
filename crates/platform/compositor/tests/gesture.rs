#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use smithay::utils::{Point, Size};
use unbound_compositor::{BACKGROUND, Gesture, Headless, Renderer};
use unbound_test_wayland::{ClientGesture, RED, ShmImage, TestClient, roundtrip};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (256, 192);
const WINDOW: i32 = 64;
const CENTER: (f64, f64) = (128.0, 96.0);
const BTN_LEFT: u32 = 0x110;

fn server() -> Headless {
    Headless::new(Size::from(OUTPUT)).expect("headless compositor")
}

fn app(server: &mut Headless) -> TestClient {
    let mut client = TestClient::connect(server);
    client.use_seat(server);
    let pixels = RED.repeat((WINDOW * WINDOW) as usize);
    client.show_window(
        server,
        &ShmImage {
            width: WINDOW,
            height: WINDOW,
            stride: WINDOW * 4,
            format: Format::Xrgb8888,
            bytes: &pixels,
        },
    );
    client.use_gestures(server);
    server.move_pointer_to(CENTER.into(), 0);
    roundtrip(server, &mut [&mut client]);
    client
}

fn pinch(server: &mut Headless, fingers: u32, scales: &[f64], start: u32) -> u32 {
    pinch_every(server, fingers, scales, start, 10)
}

fn pinch_every(server: &mut Headless, fingers: u32, scales: &[f64], start: u32, step: u32) -> u32 {
    server.gesture(Gesture::PinchBegin { fingers }, start);
    let mut time = start;
    for scale in scales {
        time += step;
        server.gesture(
            Gesture::PinchUpdate {
                delta: Point::default(),
                scale: *scale,
                rotation: 0.0,
            },
            time,
        );
    }
    time += 10;
    server.gesture(Gesture::PinchEnd { cancelled: false }, time);
    time
}

fn settle(server: &mut Headless, from: u32) {
    let mut at = Duration::from_millis(u64::from(from));
    while {
        at += Duration::from_millis(16);
        server.animate(at);
        server.is_animating()
    } {}
}

fn center(renderer: &mut Renderer, server: &mut Headless) -> [u8; 3] {
    renderer.render(server).expect("frame");
    let snapshot = renderer.snapshot(server).expect("read back");
    let at = ((80 * snapshot.size.w + 110) * 4) as usize;
    [
        snapshot.rgba[at],
        snapshot.rgba[at + 1],
        snapshot.rgba[at + 2],
    ]
}

#[test]
fn two_and_three_finger_gestures_and_holds_reach_the_app_under_the_pointer() {
    let mut server = server();
    let mut client = app(&mut server);

    server.gesture(Gesture::SwipeBegin { fingers: 3 }, 1);
    server.gesture(
        Gesture::SwipeUpdate {
            delta: (4.0, -2.0).into(),
        },
        2,
    );
    server.gesture(Gesture::SwipeEnd { cancelled: false }, 3);
    pinch(&mut server, 2, &[1.5], 10);
    server.gesture(Gesture::HoldBegin { fingers: 2 }, 40);
    server.gesture(Gesture::HoldEnd { cancelled: true }, 50);
    client.exchange(&mut server);

    assert_eq!(
        client.gestures(),
        [
            ClientGesture::Swipe { fingers: 3 },
            ClientGesture::SwipeMoved { dx: 4.0, dy: -2.0 },
            ClientGesture::SwipeEnded { cancelled: false },
            ClientGesture::Pinch { fingers: 2 },
            ClientGesture::PinchMoved { scale: 1.5 },
            ClientGesture::PinchEnded { cancelled: false },
            ClientGesture::Hold { fingers: 2 },
            ClientGesture::HoldEnded { cancelled: true },
        ]
    );
}

#[test]
fn a_four_finger_spread_pushes_windows_aside_and_a_pinch_brings_them_back() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut client = app(&mut server);
    let covered = center(&mut renderer, &mut server);

    let end = pinch(&mut server, 4, &[1.1, 1.3, 1.5, 1.7], 100);
    settle(&mut server, end);
    let aside = center(&mut renderer, &mut server);
    let shown = server.shows_desktop();
    client.exchange(&mut server);
    let typing_while_shown = client.keyboard_focus_on(0);
    server.move_pointer_to(CENTER.into(), end + 1);
    server.press_button(BTN_LEFT, true, end + 2);
    server.press_button(BTN_LEFT, false, end + 3);
    client.exchange(&mut server);
    let clicked_through = client.pointer_focus();

    let end = pinch(&mut server, 4, &[0.9, 0.7, 0.5], end + 100);
    settle(&mut server, end);
    let back = center(&mut renderer, &mut server);
    client.exchange(&mut server);

    assert_eq!(covered, [0xff, 0, 0]);
    assert!(shown);
    assert_eq!(aside, BACKGROUND);
    assert_eq!(clicked_through, None);
    assert!(!server.shows_desktop());
    assert_eq!(back, [0xff, 0, 0]);
    assert!(!typing_while_shown);
    assert!(client.keyboard_focus_on(0));
    assert!(client.gestures().is_empty());
}

#[test]
fn a_small_slow_spread_springs_back() {
    let mut server = server();
    let _client = app(&mut server);

    let end = pinch_every(&mut server, 4, &[1.02, 1.04, 1.05, 1.06, 1.06], 100, 150);
    settle(&mut server, end);

    assert!(!server.shows_desktop());
}

#[test]
fn a_four_finger_pinch_opens_the_launcher() {
    let mut server = server();
    let mut client = app(&mut server);
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_shell(&mut server);

    pinch(&mut server, 4, &[0.9, 0.7, 0.6], 100);
    roundtrip(&mut server, &mut [&mut client, &mut shell]);

    assert_eq!(shell.launcher_requests(), 1);
    assert!(!server.shows_desktop());
}

#[test]
fn activating_an_app_while_the_desktop_shows_brings_the_windows_back() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut client = app(&mut server);
    client.set_app_id(&mut server, 0, "org.example.Editor");
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_shell(&mut server);
    let end = pinch(&mut server, 4, &[1.2, 1.5, 1.8], 100);
    settle(&mut server, end);

    shell.activate_app(&mut server, "org.example.Editor");
    settle(&mut server, end + 100);

    assert!(!server.shows_desktop());
    assert_eq!(center(&mut renderer, &mut server), [0xff, 0, 0]);
}
