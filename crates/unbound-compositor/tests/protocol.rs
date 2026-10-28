use std::time::Duration;

use smithay::utils::{Point, Size};
use unbound_compositor::Headless;
use unbound_test_wayland::{RED, ShmImage, TestClient, quadrants};
use wayland_client::protocol::wl_shm::Format;

fn server() -> Headless {
    Headless::new(Size::from((256, 192))).expect("headless compositor")
}

#[test]
fn argb_toplevel_is_mapped_centered_with_its_pixels() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    let pixels = quadrants(96, 64, 96 * 4);
    client.show_window(
        &mut server,
        &ShmImage {
            width: 96,
            height: 64,
            stride: 96 * 4,
            format: Format::Argb8888,
            bytes: &pixels,
        },
    );

    let [window] = server.windows() else {
        panic!("expected one window, got {}", server.windows().len());
    };
    assert_eq!(window.location(), Point::from((80, 64)));
    let frame = window.frame().expect("mapped frame");
    assert_eq!(frame.size, Size::from((96, 64)));
    assert_eq!(frame.bgra_premultiplied, pixels);
}

#[test]
fn xrgb_buffer_ignores_the_alpha_byte() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    let transparent_red = [RED[0], RED[1], RED[2], 0x00];
    let pixels = transparent_red.repeat(4 * 4);
    client.show_window(
        &mut server,
        &ShmImage {
            width: 4,
            height: 4,
            stride: 16,
            format: Format::Xrgb8888,
            bytes: &pixels,
        },
    );

    let frame = server.windows()[0].frame().expect("mapped frame");
    assert_eq!(frame.bgra_premultiplied, RED.repeat(4 * 4));
}

#[test]
fn row_padding_beyond_the_width_is_not_part_of_the_frame() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    let stride = 8 * 4 + 12;
    let padded = quadrants(8, 6, stride);
    client.show_window(
        &mut server,
        &ShmImage {
            width: 8,
            height: 6,
            stride: i32::try_from(stride).expect("stride"),
            format: Format::Argb8888,
            bytes: &padded,
        },
    );

    let frame = server.windows()[0].frame().expect("mapped frame");
    assert_eq!(frame.bgra_premultiplied, quadrants(8, 6, 8 * 4));
}

#[test]
fn closing_the_client_unmaps_its_window() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    let pixels = quadrants(4, 4, 16);
    client.show_window(
        &mut server,
        &ShmImage {
            width: 4,
            height: 4,
            stride: 16,
            format: Format::Argb8888,
            bytes: &pixels,
        },
    );
    assert_eq!(server.windows().len(), 1);

    drop(client);
    server.dispatch().expect("server dispatch");

    assert!(server.windows().is_empty());
}

#[test]
fn frame_callback_is_done_once_the_frame_is_presented() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    let pixels = quadrants(4, 4, 16);
    client.show_window(
        &mut server,
        &ShmImage {
            width: 4,
            height: 4,
            stride: 16,
            format: Format::Argb8888,
            bytes: &pixels,
        },
    );
    client.request_frame(&mut server);
    assert!(server.take_redraw());
    assert_eq!(client.frames_done(), 0);

    server
        .frame_done(Duration::from_millis(16))
        .expect("frame done");
    client.exchange(&mut server);

    assert_eq!(client.frames_done(), 1);
}
