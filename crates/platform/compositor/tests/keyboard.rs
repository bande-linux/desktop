#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use smithay::utils::Size;
use unbound_compositor::{BACKGROUND, Headless, KeyboardSettings, Renderer};
use unbound_config::{CapsLock, InputSource};
use unbound_test_wayland::{BLUE, RED, ShmImage, TestClient, roundtrip};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (256, 192);
const KEY_LEFTMETA: u32 = 125;
const KEY_LEFTALT: u32 = 56;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_SPACE: u32 = 57;

fn server() -> Headless {
    Headless::new(Size::from(OUTPUT)).expect("headless compositor")
}

fn settings(codes: &[&str]) -> KeyboardSettings {
    KeyboardSettings {
        input_sources: codes
            .iter()
            .map(|code| InputSource::parse(code).expect("layout"))
            .collect(),
        caps_lock: CapsLock::CapsLock,
        repeat_rate: 40,
        repeat_delay: Duration::from_millis(250),
    }
}

fn focused_app(server: &mut Headless) -> TestClient {
    let mut app = TestClient::connect(server);
    app.use_seat(server);
    let pixels = RED.repeat(32 * 32);
    app.show_window(
        server,
        &ShmImage {
            width: 32,
            height: 32,
            stride: 32 * 4,
            format: Format::Xrgb8888,
            bytes: &pixels,
        },
    );
    roundtrip(server, &mut [&mut app]);
    assert!(app.keyboard_focus_on(0));
    app
}

fn shell(server: &mut Headless) -> TestClient {
    let mut shell = TestClient::connect_as_shell(server);
    shell.join_shell(server);
    shell
}

fn switch(server: &mut Headless, backwards: bool) {
    server.key(KEY_LEFTMETA, true, 0);
    server.key(KEY_LEFTALT, true, 0);
    if backwards {
        server.key(KEY_LEFTSHIFT, true, 0);
    }
    server.key(KEY_SPACE, true, 0);
    server.key(KEY_SPACE, false, 0);
    if backwards {
        server.key(KEY_LEFTSHIFT, false, 0);
    }
    server.key(KEY_LEFTALT, false, 0);
    server.key(KEY_LEFTMETA, false, 0);
}

#[test]
fn the_shell_hears_the_input_sources_and_apps_get_the_key_repeat() {
    let mut server = server();
    let mut app = focused_app(&mut server);
    server.apply_keyboard_settings(settings(&["se", "us(dvorak)"]));
    let mut shell = shell(&mut server);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);

    assert_eq!(shell.input_sources(), Some(("se,us(dvorak)", 0)));
    assert_eq!(app.repeat_info(), Some((40, 250)));
    assert_eq!(server.active_input_source().as_deref(), Some("se"));
}

#[test]
fn option_super_space_returns_to_the_previous_source_and_held_walks_the_list() {
    let mut server = server();
    let mut app = focused_app(&mut server);
    server.apply_keyboard_settings(settings(&["se", "us", "de(nodeadkeys)"]));
    let mut shell = shell(&mut server);

    switch(&mut server, false);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);
    assert_eq!(shell.input_sources(), Some(("se,us,de(nodeadkeys)", 1)));
    assert_eq!(app.layout_group(), 1);
    assert!(app.keys().iter().all(|(key, _)| *key != KEY_SPACE));

    shell.select_input_source(&mut server, 2);
    switch(&mut server, false);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);
    assert_eq!(server.active_input_source().as_deref(), Some("us"));

    server.key(KEY_LEFTMETA, true, 0);
    server.key(KEY_LEFTALT, true, 0);
    for _ in 0..2 {
        server.key(KEY_SPACE, true, 0);
        server.key(KEY_SPACE, false, 0);
    }
    server.key(KEY_LEFTALT, false, 0);
    server.key(KEY_LEFTMETA, false, 0);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);
    assert_eq!(server.active_input_source().as_deref(), Some("se"));
    assert_eq!(app.layout_group(), 0);

    switch(&mut server, true);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);
    assert_eq!(
        server.active_input_source().as_deref(),
        Some("de(nodeadkeys)")
    );
}

#[test]
fn the_menu_bar_selects_a_source_and_a_new_list_keeps_the_active_one() {
    let mut server = server();
    server.apply_keyboard_settings(settings(&["se", "us"]));
    let mut shell = shell(&mut server);

    shell.select_input_source(&mut server, 1);
    roundtrip(&mut server, &mut [&mut shell]);
    assert_eq!(shell.input_sources(), Some(("se,us", 1)));

    server.apply_keyboard_settings(settings(&["fr", "us", "se"]));
    roundtrip(&mut server, &mut [&mut shell]);
    assert_eq!(shell.input_sources(), Some(("fr,us,se", 1)));

    shell.select_input_source(&mut server, 7);
    server.apply_keyboard_settings(settings(&["fr"]));
    roundtrip(&mut server, &mut [&mut shell]);
    assert_eq!(shell.input_sources(), Some(("fr", 0)));
}

#[test]
fn the_source_can_be_switched_while_the_session_is_locked() {
    let mut server = server();
    server.apply_keyboard_settings(settings(&["se", "us"]));
    let mut lock = TestClient::connect_as_shell(&mut server);
    lock.use_seat(&mut server);
    lock.join_shell(&mut server);
    lock.lock_session(&mut server);
    lock.show_lock_surface(&mut server, BLUE);
    roundtrip(&mut server, &mut [&mut lock]);
    assert!(lock.lock_surface_has_keyboard());

    switch(&mut server, false);
    roundtrip(&mut server, &mut [&mut lock]);

    assert_eq!(lock.input_sources(), Some(("se,us", 1)));
    assert_eq!(lock.layout_group(), 1);
    assert!(lock.keys().iter().all(|(key, _)| *key != KEY_SPACE));
}

fn centre(renderer: &mut Renderer, server: &mut Headless) -> [u8; 3] {
    renderer.render(server).expect("frame");
    let snapshot = renderer.snapshot(server).expect("read back");
    let at = ((OUTPUT.1 / 2 * snapshot.size.w + OUTPUT.0 / 2) * 4) as usize;
    [
        snapshot.rgba[at],
        snapshot.rgba[at + 1],
        snapshot.rgba[at + 2],
    ]
}

#[test]
fn switching_shows_the_new_source_in_the_middle_of_the_screen() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    server.apply_keyboard_settings(settings(&["se", "us"]));
    assert_eq!(centre(&mut renderer, &mut server), BACKGROUND);

    switch(&mut server, false);

    assert_ne!(centre(&mut renderer, &mut server), BACKGROUND);
    assert!(server.is_animating());
}
