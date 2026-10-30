use std::time::Duration;

use crate::support::*;
use unbound_compositor::{BACKGROUND, Headless, KeyboardSettings, Renderer};
use unbound_config::{CapsLock, InputSource};
use unbound_test_wayland::{BLUE, TestClient, roundtrip};

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

#[test]
fn switching_shows_the_new_source_in_the_middle_of_the_screen() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    server.apply_keyboard_settings(settings(&["se", "us"]));
    assert_eq!(
        pixel(&mut renderer, &mut server, OUTPUT.0 / 2, OUTPUT.1 / 2),
        BACKGROUND
    );

    switch(&mut server, false);

    assert_ne!(
        pixel(&mut renderer, &mut server, OUTPUT.0 / 2, OUTPUT.1 / 2),
        BACKGROUND
    );
    assert!(server.is_animating());
}
