use std::time::Duration;

use crate::support::*;
use smithay::utils::{Point, Size};
use unbound_compositor::{Compositor, OutputConfig, Renderer};
use unbound_test_wayland::{BLUE, GREEN, RED, ShmImage, TestClient, roundtrip};
use wayland_client::protocol::wl_data_device_manager::DndAction;
use wayland_client::protocol::wl_shm::Format;

const WINDOW: i32 = 96;
const ICON: i32 = 16;
const START: (i32, i32) = (128, 96);
const AWAY: (i32, i32) = (40, 40);
const DRAWN_BLUE: [u8; 3] = [0, 0, 0xff];

fn at((x, y): (i32, i32)) -> Point<f64, smithay::utils::Logical> {
    (f64::from(x), f64::from(y)).into()
}

fn image(color: [u8; 4], side: i32) -> Vec<u8> {
    color.repeat((side * side) as usize)
}

fn shm(pixels: &[u8], side: i32) -> ShmImage<'_> {
    ShmImage {
        width: side,
        height: side,
        stride: side * 4,
        format: Format::Argb8888,
        bytes: pixels,
    }
}

fn start_drag(accepts: bool) -> (Compositor, TestClient) {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut client = TestClient::connect(&mut server);
    client.use_seat(&mut server);
    client.join_seat(&mut server);
    client.accept_drops(accepts);
    let window = image(RED, WINDOW);
    client.show_window(&mut server, &shm(&window, WINDOW));
    server.move_pointer_to(at(START), 1);
    server.press_button(BTN_LEFT, true, 2);
    roundtrip(&mut server, &mut [&mut client]);
    let icon = image(BLUE, ICON);
    client.start_drag(&mut server, &shm(&icon, ICON), (-ICON / 2, -ICON / 2));
    (server, client)
}

fn icon_pixel(renderer: &mut Renderer, server: &mut Compositor, pointer: (i32, i32)) -> [u8; 3] {
    renderer.render(server).expect("frame");
    let snapshot = renderer.snapshot(server).expect("read back");
    let (x, y) = (pointer.0 - ICON / 2 + 2, pointer.1 - ICON / 2 + 2);
    let index = ((y * snapshot.size.w + x) * 4) as usize;
    [
        snapshot.rgba[index],
        snapshot.rgba[index + 1],
        snapshot.rgba[index + 2],
    ]
}

#[test]
fn the_drag_icon_follows_the_pointer_at_its_hotspot() {
    let (mut server, mut client) = start_drag(true);
    let mut renderer = Renderer::new().expect("renderer");

    server.move_pointer_to(at(AWAY), 10);
    client.exchange(&mut server);

    assert_eq!(icon_pixel(&mut renderer, &mut server, AWAY), DRAWN_BLUE);
}

#[test]
fn holding_option_turns_a_drop_into_a_copy() {
    let actions = |alt: bool| {
        let (mut server, mut client) = start_drag(true);
        if alt {
            server.key(KEY_LEFTALT, true, 5);
        }
        server.move_pointer_to(at(AWAY), 10);
        server.move_pointer_to(at(START), 20);
        client.exchange(&mut server);
        client.drag_actions().last().copied()
    };

    assert_eq!(actions(false), Some(DndAction::Move));
    assert_eq!(actions(true), Some(DndAction::Copy));
}

#[test]
fn a_refused_drop_slides_the_icon_back_to_where_the_drag_began() {
    let (mut server, mut client) = start_drag(false);
    let mut renderer = Renderer::new().expect("renderer");
    server.move_pointer_to(at(AWAY), 10);
    let dragged = icon_pixel(&mut renderer, &mut server, AWAY);
    server.press_button(BTN_LEFT, false, 20);
    client.exchange(&mut server);

    let mut time = Duration::from_secs(1);
    server.animate(time);
    let at_drop = icon_pixel(&mut renderer, &mut server, AWAY);
    let mut reached_start = false;
    while server.is_animating() {
        time += Duration::from_millis(16);
        server.animate(time);
        reached_start |= icon_pixel(&mut renderer, &mut server, START) == DRAWN_BLUE;
    }

    assert_eq!(dragged, DRAWN_BLUE);
    assert_eq!(at_drop, DRAWN_BLUE);
    assert!(reached_start);
    assert_ne!(icon_pixel(&mut renderer, &mut server, START), DRAWN_BLUE);
    assert_eq!(client.cancelled_sources(), 1);
    assert_eq!(client.drops(), 0);
}

#[test]
fn escape_cancels_a_drag_slides_the_icon_back_and_nothing_is_dropped() {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut target = TestClient::connect(&mut server);
    target.use_seat(&mut server);
    target.join_seat(&mut server);
    target.accept_drops(true);
    let behind = image(GREEN, WINDOW);
    target.show_window(&mut server, &shm(&behind, WINDOW));
    target.set_maximized(&mut server, 0, true);
    target.fill_configured_size(&mut server, 0, GREEN);
    let mut source = TestClient::connect(&mut server);
    source.use_seat(&mut server);
    source.join_seat(&mut server);
    let window = image(RED, WINDOW);
    source.show_window(&mut server, &shm(&window, WINDOW));
    server.move_pointer_to(at(START), 1);
    server.press_button(BTN_LEFT, true, 2);
    roundtrip(&mut server, &mut [&mut target, &mut source]);
    let icon = image(BLUE, ICON);
    source.start_drag(&mut server, &shm(&icon, ICON), (-ICON / 2, -ICON / 2));
    let mut renderer = Renderer::new().expect("renderer");
    server.move_pointer_to(at(AWAY), 10);
    roundtrip(&mut server, &mut [&mut target, &mut source]);
    assert_eq!(icon_pixel(&mut renderer, &mut server, AWAY), DRAWN_BLUE);

    server.key(KEY_ESC, true, 20);
    server.key(KEY_ESC, false, 21);
    roundtrip(&mut server, &mut [&mut target, &mut source]);

    assert_eq!(source.cancelled_sources(), 1);
    assert_eq!(target.drag_leaves(), 1);
    assert!(server.is_animating());
    assert!(source.keys().is_empty() && target.keys().is_empty());
    let mut time = Duration::from_secs(1);
    let mut reached_start = false;
    while server.is_animating() {
        time += Duration::from_millis(16);
        server.animate(time);
        reached_start |= icon_pixel(&mut renderer, &mut server, START) == DRAWN_BLUE;
    }
    assert!(reached_start);

    let pressed = target.buttons().len();
    server.move_pointer_to(at((AWAY.0 + 5, AWAY.1)), 30);
    server.press_button(BTN_LEFT, false, 31);
    roundtrip(&mut server, &mut [&mut target, &mut source]);
    assert_eq!(target.drops(), 0);
    assert_eq!(target.buttons().len(), pressed);
    assert!(!server.pointer_grabbed());

    server.press_button(BTN_LEFT, true, 40);
    server.press_button(BTN_LEFT, false, 41);
    roundtrip(&mut server, &mut [&mut target, &mut source]);
    assert_eq!(target.buttons().len(), pressed + 2);
}

#[test]
fn pressing_option_or_super_mid_drag_changes_the_action_without_moving() {
    let (mut server, mut client) = start_drag(true);
    server.move_pointer_to(at(AWAY), 10);
    server.move_pointer_to(at(START), 20);
    client.exchange(&mut server);
    let resting = client.drag_actions().last().copied();

    server.key(KEY_LEFTALT, true, 30);
    client.exchange(&mut server);
    let with_option = client.drag_actions().last().copied();
    server.key(KEY_LEFTALT, false, 31);
    client.exchange(&mut server);
    let released = client.drag_actions().last().copied();

    assert_eq!(resting, Some(DndAction::Move));
    assert_eq!(with_option, Some(DndAction::Copy));
    assert_eq!(released, Some(DndAction::Move));
}

fn two_clients() -> (Compositor, TestClient, TestClient) {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut target = TestClient::connect(&mut server);
    target.use_seat(&mut server);
    target.join_seat(&mut server);
    target.accept_drops(true);
    let behind = image(GREEN, WINDOW);
    target.show_window(&mut server, &shm(&behind, WINDOW));
    target.set_maximized(&mut server, 0, true);
    target.fill_configured_size(&mut server, 0, GREEN);
    let mut source = TestClient::connect(&mut server);
    source.use_seat(&mut server);
    source.join_seat(&mut server);
    let window = image(RED, WINDOW);
    source.show_window(&mut server, &shm(&window, WINDOW));
    server.move_pointer_to(at(START), 1);
    server.press_button(BTN_LEFT, true, 2);
    roundtrip(&mut server, &mut [&mut target, &mut source]);
    let icon = image(BLUE, ICON);
    source.start_drag(&mut server, &shm(&icon, ICON), (-ICON / 2, -ICON / 2));
    server.move_pointer_to(at(AWAY), 10);
    roundtrip(&mut server, &mut [&mut target, &mut source]);
    (server, target, source)
}

fn next_click_reaches(server: &mut Compositor, client: &mut TestClient) {
    assert!(!server.pointer_grabbed());
    let pressed = client.buttons().len();
    server.move_pointer_to(at((AWAY.0 + 3, AWAY.1)), 50);
    server.press_button(BTN_LEFT, true, 51);
    server.press_button(BTN_LEFT, false, 52);
    client.exchange(server);
    assert_eq!(client.buttons().len(), pressed + 2);
}

#[test]
fn a_source_that_dies_mid_drag_ends_the_drag_and_frees_the_pointer() {
    let (mut server, mut target, source) = two_clients();
    assert!(server.pointer_grabbed());

    drop(source);
    roundtrip(&mut server, &mut [&mut target]);

    assert!(!server.pointer_grabbed());
    server.press_button(BTN_LEFT, false, 40);
    roundtrip(&mut server, &mut [&mut target]);
    next_click_reaches(&mut server, &mut target);
}

#[test]
fn a_target_that_dies_mid_drag_gets_no_drop_and_the_source_is_cancelled() {
    let (mut server, target, mut source) = two_clients();

    drop(target);
    roundtrip(&mut server, &mut [&mut source]);
    server.press_button(BTN_LEFT, false, 40);
    roundtrip(&mut server, &mut [&mut source]);

    assert_eq!(source.cancelled_sources(), 1);
    assert!(!server.pointer_grabbed());
}

#[test]
fn the_drag_icon_is_drawn_on_both_displays_where_it_straddles_the_seam() {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    server.add_output(OutputConfig::virtual_display(
        "HEADLESS-2",
        Size::from(OUTPUT),
    ));
    let mut client = TestClient::connect(&mut server);
    client.use_seat(&mut server);
    client.join_seat(&mut server);
    let window = image(RED, WINDOW);
    client.show_window(&mut server, &shm(&window, WINDOW));
    server.move_pointer_to(at(START), 1);
    server.press_button(BTN_LEFT, true, 2);
    roundtrip(&mut server, &mut [&mut client]);
    let icon = image(BLUE, ICON);
    client.start_drag(&mut server, &shm(&icon, ICON), (-ICON / 2, -ICON / 2));
    let mut renderer = Renderer::new().expect("renderer");

    server.move_pointer_to(at((OUTPUT.0 - 2, START.1)), 10);
    client.exchange(&mut server);
    let mut blue_on = |output: &str, x: i32| {
        renderer.render_output(&mut server, output).expect("frame");
        let snapshot = renderer.snapshot_output(output).expect("read back");
        let index = ((START.1 * snapshot.size.w + x) * 4) as usize;
        snapshot.rgba[index..index + 3] == DRAWN_BLUE
    };

    assert!(blue_on("HEADLESS-1", OUTPUT.0 - 6));
    assert!(blue_on("HEADLESS-2", 2));
}
