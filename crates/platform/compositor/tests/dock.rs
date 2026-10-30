#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use smithay::utils::{Point, Size};
use unbound_compositor::{BACKGROUND, Headless, Renderer};
use unbound_test_wayland::{
    Anchor, KeyboardInteractivity, Layer, LayerRequest, RED, ShmImage, TestClient, roundtrip,
};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (256, 192);
const WINDOW: i32 = 64;
const EDITOR: &str = "org.example.Editor";
const KEY_LEFTMETA: u32 = 125;
const KEY_M: u32 = 50;
const KEY_H: u32 = 35;
const KEY_F2: u32 = 60;
const KEY_F3: u32 = 61;
const BTN_LEFT: u32 = 0x110;

fn server() -> Headless {
    Headless::new(Size::from(OUTPUT)).expect("headless compositor")
}

fn app(server: &mut Headless, app_id: &str) -> TestClient {
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
    client.set_app_id(server, 0, app_id);
    client
}

fn shell(server: &mut Headless) -> TestClient {
    let mut shell = TestClient::connect_as_shell(server);
    shell.use_seat(server);
    shell.join_shell(server);
    shell
}

fn settle(server: &mut Headless) {
    let mut at = Duration::from_secs(1);
    while {
        at += Duration::from_millis(16);
        server.animate(at);
        server.is_animating()
    } {}
}

fn window_pixel(renderer: &mut Renderer, server: &mut Headless) -> [u8; 3] {
    renderer.render(server).expect("frame");
    let snapshot = renderer.snapshot(server).expect("read back");
    let at = ((80 * snapshot.size.w + 110) * 4) as usize;
    [
        snapshot.rgba[at],
        snapshot.rgba[at + 1],
        snapshot.rgba[at + 2],
    ]
}

fn super_key(server: &mut Headless, key: u32) {
    server.key(KEY_LEFTMETA, true, 1);
    server.key(key, true, 2);
    server.key(key, false, 3);
    server.key(KEY_LEFTMETA, false, 4);
}

#[test]
fn a_hidden_app_fades_out_loses_the_keyboard_and_comes_back_when_activated() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = app(&mut server, EDITOR);
    let mut dock = shell(&mut server);
    settle(&mut server);

    dock.hide_app(&mut server, EDITOR);
    settle(&mut server);
    editor.exchange(&mut server);
    let hidden = window_pixel(&mut renderer, &mut server);
    let typing_while_hidden = editor.keyboard_focus_on(0);
    dock.activate_app(&mut server, EDITOR);
    settle(&mut server);
    editor.exchange(&mut server);

    assert_eq!(hidden, BACKGROUND);
    assert!(!typing_while_hidden);
    assert_eq!(window_pixel(&mut renderer, &mut server), [0xff, 0, 0]);
    assert!(editor.keyboard_focus_on(0));
}

#[test]
fn super_h_hides_the_focused_app_and_show_app_brings_it_back_without_the_keyboard() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = app(&mut server, EDITOR);
    let mut menu_bar = shell(&mut server);
    settle(&mut server);

    super_key(&mut server, KEY_H);
    settle(&mut server);
    editor.exchange(&mut server);
    let hidden = window_pixel(&mut renderer, &mut server);
    menu_bar.show_app(&mut server, EDITOR);
    settle(&mut server);
    editor.exchange(&mut server);

    assert_eq!(hidden, BACKGROUND);
    assert_eq!(window_pixel(&mut renderer, &mut server), [0xff, 0, 0]);
    assert!(!editor.keyboard_focus_on(0));
}

#[test]
fn quitting_an_app_from_the_dock_asks_its_windows_to_close() {
    let mut server = server();
    let mut editor = app(&mut server, EDITOR);
    let mut dock = shell(&mut server);

    dock.quit_app(&mut server, EDITOR);
    editor.exchange(&mut server);

    assert!(editor.close_requested(0));
}

#[test]
fn the_dock_learns_which_windows_are_minimised_and_can_bring_one_back() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = app(&mut server, EDITOR);
    let mut dock = shell(&mut server);
    roundtrip(&mut server, &mut [&mut editor, &mut dock]);
    let identifier = dock
        .window_identifiers(EDITOR)
        .pop()
        .expect("a listed window");

    super_key(&mut server, KEY_M);
    settle(&mut server);
    dock.exchange(&mut server);
    let minimized = window_pixel(&mut renderer, &mut server);
    let late = shell(&mut server);
    dock.activate_window(&mut server, &identifier);
    settle(&mut server);
    roundtrip(&mut server, &mut [&mut editor, &mut dock]);

    assert_eq!(minimized, BACKGROUND);
    assert_eq!(
        dock.minimized_windows(),
        [(identifier.clone(), true), (identifier.clone(), false)]
    );
    assert_eq!(late.minimized_windows(), [(identifier, true)]);
    assert_eq!(window_pixel(&mut renderer, &mut server), [0xff, 0, 0]);
    assert!(editor.keyboard_focus_on(0));
}

#[test]
fn an_app_that_asks_to_come_forward_without_a_valid_token_asks_for_attention_instead() {
    let mut server = server();
    let mut background = app(&mut server, "org.example.Mail");
    let mut editor = app(&mut server, EDITOR);
    let mut dock = shell(&mut server);
    roundtrip(&mut server, &mut [&mut background, &mut editor, &mut dock]);

    let token = background.activation_token(&mut server);
    background.activate(&mut server, 0, &token);
    roundtrip(&mut server, &mut [&mut background, &mut editor, &mut dock]);

    assert_eq!(dock.attention_requests(), ["org.example.Mail"]);
    assert!(editor.keyboard_focus_on(0));
}

#[test]
fn super_f3_gives_the_dock_the_keyboard_and_super_f2_the_menu_bar() {
    let mut server = server();
    let mut editor = app(&mut server, EDITOR);
    let mut dock = shell(&mut server);
    let bar = |anchor, size| LayerRequest {
        layer: Layer::Top,
        anchor,
        size,
        exclusive_zone: 0,
        keyboard: KeyboardInteractivity::OnDemand,
        color: [0x80; 4],
    };
    dock.show_named_layer(&mut server, &bar(Anchor::Bottom, (128, 24)), "dock");
    dock.show_named_layer(
        &mut server,
        &bar(Anchor::Top | Anchor::Left | Anchor::Right, (0, 16)),
        "menubar",
    );

    super_key(&mut server, KEY_F3);
    roundtrip(&mut server, &mut [&mut editor, &mut dock]);
    let dock_has_keyboard = dock.layer_has_keyboard(0);
    super_key(&mut server, KEY_F2);
    roundtrip(&mut server, &mut [&mut editor, &mut dock]);

    assert!(dock_has_keyboard);
    assert!(dock.layer_has_keyboard(1));
    assert!(!editor.keyboard_focus_on(0));
    assert_eq!(dock.shell_focus_requests(), ["dock", "menubar"]);
}

#[test]
fn the_combined_bar_takes_the_keyboard_for_both_super_f2_and_super_f3() {
    let mut server = server();
    let mut editor = app(&mut server, EDITOR);
    let mut bar = shell(&mut server);
    bar.show_named_layer(
        &mut server,
        &LayerRequest {
            layer: Layer::Top,
            anchor: Anchor::Top | Anchor::Left | Anchor::Bottom,
            size: (64, 0),
            exclusive_zone: 64,
            keyboard: KeyboardInteractivity::OnDemand,
            color: [0x80; 4],
        },
        "bar",
    );

    super_key(&mut server, KEY_F3);
    roundtrip(&mut server, &mut [&mut editor, &mut bar]);
    let bar_has_keyboard = bar.layer_has_keyboard(0);
    super_key(&mut server, KEY_F2);
    roundtrip(&mut server, &mut [&mut editor, &mut bar]);

    assert!(bar_has_keyboard);
    assert!(bar.layer_has_keyboard(0));
    assert_eq!(bar.shell_focus_requests(), ["dock", "menubar"]);
}

#[test]
fn clicking_the_bar_keeps_the_active_app() {
    let mut server = server();
    let mut editor = app(&mut server, EDITOR);
    let mut bar = shell(&mut server);
    bar.show_named_layer(
        &mut server,
        &LayerRequest {
            layer: Layer::Top,
            anchor: Anchor::Top | Anchor::Left | Anchor::Bottom,
            size: (64, 0),
            exclusive_zone: 64,
            keyboard: KeyboardInteractivity::OnDemand,
            color: [0x80; 4],
        },
        "bar",
    );
    roundtrip(&mut server, &mut [&mut editor, &mut bar]);

    server.move_pointer_to(Point::from((10.0, 60.0)), 5);
    server.press_button(BTN_LEFT, true, 6);
    server.press_button(BTN_LEFT, false, 7);
    roundtrip(&mut server, &mut [&mut editor, &mut bar]);

    assert!(bar.layer_has_keyboard(0));
    assert_eq!(bar.active_apps(), [EDITOR]);
}
