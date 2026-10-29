#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use smithay::utils::{Point, Size};
use unbound_compositor::{Headless, Vblank};
use unbound_test_wayland::{ShmImage, TestClient, quadrants};
use wayland_client::protocol::wl_shm::Format;

fn server() -> Headless {
    Headless::new(Size::from((256, 192))).expect("headless compositor")
}

#[test]
fn toplevel_is_mapped_centered_at_its_buffer_size() {
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
    assert_eq!(window.size(), Some(Size::from((96, 64))));
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

    server.latch();
    server
        .present(Vblank {
            sequence: 2,
            time: Duration::from_millis(16),
        })
        .expect("presented");
    client.exchange(&mut server);

    assert_eq!(client.frames_done(), 1);
}
