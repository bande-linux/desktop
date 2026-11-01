use crate::support::*;
use bande_compositor::{BACKGROUND, Compositor, Gesture, PointerSettings, Renderer};
use bande_config::Trackpad;
use bande_test_wayland::{ClientGesture, RED, TestClient, roundtrip};
use smithay::utils::Point;

const WINDOW: i32 = 64;
const CENTER: (f64, f64) = (128.0, 96.0);

fn app(server: &mut Compositor) -> TestClient {
    let mut client = window(server, RED, (WINDOW, WINDOW));
    client.use_gestures(server);
    server.move_pointer_to(CENTER.into(), 0);
    roundtrip(server, &mut [&mut client]);
    client
}

fn pinch(server: &mut Compositor, fingers: u32, scales: &[f64], start: u32) -> u32 {
    pinch_every(server, fingers, scales, start, 10)
}

fn pinch_every(
    server: &mut Compositor,
    fingers: u32,
    scales: &[f64],
    start: u32,
    step: u32,
) -> u32 {
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

#[test]
fn two_and_four_finger_swipes_pinches_and_holds_reach_the_app_under_the_pointer() {
    let mut server = server();
    let mut client = app(&mut server);

    server.gesture(Gesture::SwipeBegin { fingers: 4 }, 1);
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
            ClientGesture::Swipe { fingers: 4 },
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
    let covered = pixel(&mut renderer, &mut server, 110, 80);

    let end = pinch(&mut server, 4, &[1.1, 1.3, 1.5, 1.7], 100);
    settle(&mut server, u64::from(end));
    let aside = pixel(&mut renderer, &mut server, 110, 80);
    let shown = server.shows_desktop();
    client.exchange(&mut server);
    let typing_while_shown = client.keyboard_focus_on(0);
    server.move_pointer_to(CENTER.into(), end + 1);
    server.press_button(BTN_LEFT, true, end + 2);
    server.press_button(BTN_LEFT, false, end + 3);
    client.exchange(&mut server);
    let clicked_through = client.pointer_focus();

    let end = pinch(&mut server, 4, &[0.9, 0.7, 0.5], end + 100);
    settle(&mut server, u64::from(end));
    let back = pixel(&mut renderer, &mut server, 110, 80);
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
    settle(&mut server, u64::from(end));

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

fn turn_off(server: &mut Compositor, trackpad: Trackpad) {
    server.apply_pointer_settings(PointerSettings {
        trackpad,
        ..PointerSettings::default()
    });
}

#[test]
fn a_three_finger_swipe_whose_gesture_is_off_reaches_the_app_and_keeps_the_workspace() {
    let mut server = server();
    let mut client = app(&mut server);
    turn_off(
        &mut server,
        Trackpad {
            workspace_swipe: false,
            ..Trackpad::default()
        },
    );

    server.gesture(Gesture::SwipeBegin { fingers: 3 }, 1);
    for (step, delta) in [(10.0, 1.0), (12.0, 0.0), (30.0, 0.0)]
        .into_iter()
        .enumerate()
    {
        server.gesture(
            Gesture::SwipeUpdate {
                delta: delta.into(),
            },
            2 + step as u32,
        );
    }
    server.gesture(Gesture::SwipeEnd { cancelled: false }, 10);
    client.exchange(&mut server);

    assert_eq!(server.active_workspace(), 0);
    assert_eq!(
        client.gestures(),
        [
            ClientGesture::Swipe { fingers: 3 },
            ClientGesture::SwipeMoved { dx: 10.0, dy: 1.0 },
            ClientGesture::SwipeMoved { dx: 12.0, dy: 0.0 },
            ClientGesture::SwipeMoved { dx: 30.0, dy: 0.0 },
            ClientGesture::SwipeEnded { cancelled: false },
        ]
    );
}

#[test]
fn a_four_finger_spread_whose_gestures_are_off_leaves_the_desktop_and_reaches_the_app() {
    let mut server = server();
    let mut client = app(&mut server);
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_shell(&mut server);
    turn_off(
        &mut server,
        Trackpad {
            desktop_spread: false,
            ..Trackpad::default()
        },
    );

    let end = pinch(&mut server, 4, &[1.2, 1.5, 1.8], 100);
    assert!(!server.shows_desktop());
    pinch(&mut server, 4, &[0.9, 0.7, 0.6], end + 100);
    roundtrip(&mut server, &mut [&mut client, &mut shell]);
    assert_eq!(shell.launcher_requests(), 1);
    assert!(client.gestures().is_empty());

    turn_off(
        &mut server,
        Trackpad {
            desktop_spread: false,
            launcher_pinch: false,
            ..Trackpad::default()
        },
    );
    pinch(&mut server, 4, &[1.5], end + 500);
    client.exchange(&mut server);

    assert!(!server.shows_desktop());
    assert_eq!(
        client.gestures(),
        [
            ClientGesture::Pinch { fingers: 4 },
            ClientGesture::PinchMoved { scale: 1.5 },
            ClientGesture::PinchEnded { cancelled: false },
        ]
    );
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
    settle(&mut server, u64::from(end));

    shell.activate_app(&mut server, "org.example.Editor");
    settle(&mut server, u64::from(end + 100));

    assert!(!server.shows_desktop());
    assert_eq!(pixel(&mut renderer, &mut server, 110, 80), [0xff, 0, 0]);
}

#[test]
fn super_d_shows_the_desktop_and_brings_the_windows_back() {
    const KEY_D: u32 = 32;
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut client = app(&mut server);
    client.exchange(&mut server);

    chord(&mut server, &[KEY_LEFTMETA, KEY_D]);
    let end = settle(&mut server, 0);
    let aside = pixel(&mut renderer, &mut server, 110, 80);
    let shown = server.shows_desktop();
    client.exchange(&mut server);
    let typing_while_shown = client.keyboard_focus_on(0);

    chord(&mut server, &[KEY_LEFTMETA, KEY_D]);
    settle(&mut server, end);
    client.exchange(&mut server);

    assert!(shown);
    assert_eq!(aside, BACKGROUND);
    assert!(!typing_while_shown);
    assert!(!server.shows_desktop());
    assert_eq!(pixel(&mut renderer, &mut server, 110, 80), [0xff, 0, 0]);
    assert!(client.keyboard_focus_on(0));
}
