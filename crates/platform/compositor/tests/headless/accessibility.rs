use std::thread;
use std::time::{Duration, Instant};

use crate::support::*;
use bande_compositor::{
    Compositor, KEYBOARD_MONITOR_NAME, Renderer, SCREEN_READER_NAME, Snapshot,
    serve_keyboard_monitor,
};
use bande_config::Accessibility;
use bande_test_bus::SessionBus;
use bande_test_wayland::roundtrip;
use smithay::utils::{Point, Size};
use zbus::blocking::MessageIterator;
use zbus::{MatchRule, message};

const WINDOW: i32 = 32;
const KEY_8: u32 = 9;
const KEY_MINUS: u32 = 12;
const KEY_EQUAL: u32 = 13;
const KEY_INSERT: u32 = 110;
const XK_A: u32 = 0x61;
const XK_INSERT: u32 = 0xff63;
const MONITOR: &str = "org.freedesktop.a11y.KeyboardMonitor";
const MONITOR_PATH: &str = "/org/freedesktop/a11y/Manager";

fn with(change: impl FnOnce(&mut Accessibility)) -> Accessibility {
    let mut settings = Accessibility::default();
    change(&mut settings);
    settings
}

fn tap(server: &mut Compositor, key: u32, time: u32) {
    server.key(key, true, time);
    server.key(key, false, time + 20);
}

fn red_pixels(snapshot: &Snapshot) -> usize {
    snapshot
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60)
        .count()
}

#[test]
fn with_sticky_keys_a_modifier_pressed_alone_holds_until_the_next_key() {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut app = focused_app(&mut server);
    server.apply_accessibility(with(|access| access.sticky_keys = true));

    tap(&mut server, KEY_LEFTSHIFT, 0);
    assert_eq!(server.latched_modifiers(), [KEY_LEFTSHIFT]);
    tap(&mut server, KEY_A, 100);
    roundtrip(&mut server, &mut [&mut app]);
    assert_eq!(
        app.keys(),
        [
            (KEY_LEFTSHIFT, true),
            (KEY_A, true),
            (KEY_A, false),
            (KEY_LEFTSHIFT, false)
        ]
    );
    assert!(server.latched_modifiers().is_empty());

    tap(&mut server, KEY_LEFTSHIFT, 200);
    tap(&mut server, KEY_LEFTSHIFT, 300);
    tap(&mut server, KEY_A, 400);
    tap(&mut server, KEY_A, 500);
    assert_eq!(server.latched_modifiers(), [KEY_LEFTSHIFT]);
    tap(&mut server, KEY_LEFTSHIFT, 600);
    roundtrip(&mut server, &mut [&mut app]);
    assert_eq!(
        &app.keys()[4..],
        [
            (KEY_LEFTSHIFT, true),
            (KEY_A, true),
            (KEY_A, false),
            (KEY_A, true),
            (KEY_A, false),
            (KEY_LEFTSHIFT, false)
        ]
    );
    assert!(server.latched_modifiers().is_empty());
}

#[test]
fn turning_sticky_keys_off_releases_a_latched_modifier() {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut app = focused_app(&mut server);
    server.apply_accessibility(with(|access| access.sticky_keys = true));
    tap(&mut server, KEY_LEFTSHIFT, 0);
    server.apply_accessibility(Accessibility::default());
    roundtrip(&mut server, &mut [&mut app]);
    assert_eq!(app.keys(), [(KEY_LEFTSHIFT, true), (KEY_LEFTSHIFT, false)]);
}

#[test]
fn bounce_keys_ignore_a_key_pressed_again_right_after_its_release() {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut app = focused_app(&mut server);
    server.apply_accessibility(with(|access| access.bounce_keys = true));

    tap(&mut server, KEY_A, 0);
    tap(&mut server, KEY_A, 100);
    tap(&mut server, KEY_A, 600);
    roundtrip(&mut server, &mut [&mut app]);
    assert_eq!(
        app.keys(),
        [(KEY_A, true), (KEY_A, false), (KEY_A, true), (KEY_A, false)]
    );
}

#[test]
fn slow_keys_accept_only_a_key_held_long_enough() {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut app = focused_app(&mut server);
    server.apply_accessibility(with(|access| access.slow_keys = true));

    tap(&mut server, KEY_A, 0);
    roundtrip(&mut server, &mut [&mut app]);
    assert!(app.keys().is_empty());

    server.key(KEY_A, true, 1000);
    let held_until = Instant::now() + Duration::from_millis(450);
    while Instant::now() < held_until {
        server.dispatch().expect("dispatch");
        thread::sleep(Duration::from_millis(5));
    }
    roundtrip(&mut server, &mut [&mut app]);
    assert_eq!(app.keys(), [(KEY_A, true)]);
    server.key(KEY_A, false, 1500);
    roundtrip(&mut server, &mut [&mut app]);
    assert_eq!(app.keys(), [(KEY_A, true), (KEY_A, false)]);
}

#[test]
fn zoom_magnifies_around_the_pointer_and_steps_with_the_keyboard() {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut app = focused_app(&mut server);
    let mut renderer = Renderer::new().expect("renderer");
    server.set_reduce_motion(true);
    let window = server.windows()[0].location();
    server.move_pointer_to(
        Point::from((
            f64::from(window.x + WINDOW / 2),
            f64::from(window.y + WINDOW / 2),
        )),
        0,
    );
    roundtrip(&mut server, &mut [&mut app]);
    renderer.render(&mut server).expect("frame");
    let unzoomed = red_pixels(&renderer.snapshot(&server).expect("read back"));

    chord(&mut server, &[KEY_LEFTMETA, KEY_LEFTALT, KEY_8]);
    assert_eq!(server.zoom_level(), 2.0);
    assert_eq!(server.render_scale(&server.main_output()), Some(2.0));
    renderer.render(&mut server).expect("frame");
    let zoomed = red_pixels(&renderer.snapshot(&server).expect("read back"));
    assert!(
        zoomed.abs_diff(unzoomed * 4) <= unzoomed / 10,
        "{unzoomed} red pixels became {zoomed}"
    );

    chord(&mut server, &[KEY_LEFTMETA, KEY_LEFTALT, KEY_EQUAL]);
    assert_eq!(server.zoom_level(), 2.5);
    chord(&mut server, &[KEY_LEFTMETA, KEY_LEFTALT, KEY_MINUS]);
    chord(&mut server, &[KEY_LEFTMETA, KEY_LEFTALT, KEY_MINUS]);
    assert_eq!(server.zoom_level(), 1.6);
    chord(&mut server, &[KEY_LEFTMETA, KEY_LEFTALT, KEY_8]);
    assert_eq!(server.zoom_level(), 1.0);
    chord(&mut server, &[KEY_LEFTMETA, KEY_LEFTALT, KEY_8]);
    assert_eq!(server.zoom_level(), 1.6);
    roundtrip(&mut server, &mut [&mut app]);
    assert!(app.keys().iter().all(|(key, _)| *key != KEY_8));
}

#[test]
fn zoom_glides_to_its_level_unless_motion_is_reduced() {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    server.apply_accessibility(with(|access| access.zoom = 3.0));
    server.animate(Duration::from_millis(16));
    server.animate(Duration::from_millis(32));
    let gliding = server.zoom_level();
    assert!(gliding > 1.0 && gliding < 3.0, "{gliding}");
    assert!(server.is_animating());
    server.animate(Duration::from_secs(2));
    assert_eq!(server.zoom_level(), 3.0);
}

#[test]
fn an_alert_flashes_the_screen_only_when_asked_to() {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut renderer = Renderer::new().expect("renderer");
    renderer.render(&mut server).expect("frame");
    let calm = renderer.snapshot(&server).expect("read back").rgba[0];

    server.flash_screen();
    assert!(!server.is_animating());

    server.apply_accessibility(with(|access| access.flash_screen = true));
    server.flash_screen();
    renderer.render(&mut server).expect("frame");
    let flashed = renderer.snapshot(&server).expect("read back").rgba[0];
    assert!(flashed > calm.saturating_add(60), "{calm} → {flashed}");
    server.animate(Duration::ZERO);
    server.animate(Duration::from_secs(3));
    assert!(!server.is_animating());
}

#[test]
fn a_screen_reader_hears_every_key_and_keeps_its_own_commands_from_the_app() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut app = focused_app(&mut server);
    let handle = serve_keyboard_monitor(&bus.connect().expect("connection")).expect("monitor");
    server.monitor_keyboard(handle);
    let stranger = bus.connect().expect("other connection");
    let refused = stranger.call_method(
        Some(KEYBOARD_MONITOR_NAME),
        MONITOR_PATH,
        Some(MONITOR),
        "GrabKeyboard",
        &(),
    );
    assert!(
        refused.is_err(),
        "only the screen reader may grab the keyboard"
    );
    let reader = bus.connect().expect("screen reader connection");
    reader
        .request_name(SCREEN_READER_NAME)
        .expect("screen reader name");
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .interface(MONITOR)
        .expect("interface")
        .member("KeyEvent")
        .expect("member")
        .build();
    let mut heard = MessageIterator::for_match_rule(rule, &reader, None).expect("subscribe");
    let monitor = |method: &str, body: &(Vec<u32>, Vec<(u32, u32)>)| {
        let reply = match method {
            "SetKeyGrabs" => reader.call_method(
                Some(KEYBOARD_MONITOR_NAME),
                MONITOR_PATH,
                Some(MONITOR),
                method,
                body,
            ),
            _ => reader.call_method(
                Some(KEYBOARD_MONITOR_NAME),
                MONITOR_PATH,
                Some(MONITOR),
                method,
                &(),
            ),
        };
        reply.expect(method);
    };
    monitor("WatchKeyboard", &(Vec::new(), Vec::new()));
    monitor("SetKeyGrabs", &(vec![XK_INSERT], Vec::new()));

    tap(&mut server, KEY_A, 0);
    server.key(KEY_INSERT, true, 100);
    tap(&mut server, KEY_A, 200);
    server.key(KEY_INSERT, false, 300);
    roundtrip(&mut server, &mut [&mut app]);
    assert_eq!(app.keys(), [(KEY_A, true), (KEY_A, false)]);

    let mut events = Vec::new();
    while events.len() < 6 {
        let signal = heard.next().expect("a key event").expect("signal");
        let (released, _, keysym, _, _): (bool, u32, u32, u32, u16) =
            signal.body().deserialize().expect("(buuuq)");
        events.push((keysym, released));
    }
    assert_eq!(
        events,
        [
            (XK_A, false),
            (XK_A, true),
            (XK_INSERT, false),
            (XK_A, false),
            (XK_A, true),
            (XK_INSERT, true)
        ]
    );

    monitor("GrabKeyboard", &(Vec::new(), Vec::new()));
    tap(&mut server, KEY_A, 400);
    roundtrip(&mut server, &mut [&mut app]);
    assert_eq!(app.keys().len(), 2);
    drop(heard);
    drop(reader);
    let released = Instant::now() + Duration::from_secs(5);
    while app.keys().len() == 2 && Instant::now() < released {
        thread::sleep(Duration::from_millis(20));
        tap(&mut server, KEY_A, 500);
        roundtrip(&mut server, &mut [&mut app]);
    }
    assert!(
        app.keys().len() > 2,
        "a vanished screen reader still holds the keyboard"
    );
}
