use crate::support::*;
use unbound_test_wayland::{LINEAR, RED, ShmImage, TestClient};
use wayland_client::protocol::wl_shm::Format;
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::Error;

const INTEL_X_TILED: u64 = 0x0100_0000_0000_0001;

#[test]
fn a_dmabuf_whose_rows_run_past_its_file_is_out_of_bounds() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    let image = ShmImage {
        width: 4,
        height: 1024,
        stride: 16,
        format: Format::Xrgb8888,
        bytes: &[0; 16],
    };

    client.create_dmabuf(&mut server, &image, LINEAR);

    assert_eq!(client.protocol_error(), Some(Error::OutOfBounds as u32));
}

#[test]
fn an_immediate_dmabuf_with_a_modifier_that_was_not_offered_is_an_invalid_buffer() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);

    let pixels = solid(RED, (4, 4));

    client.create_dmabuf(&mut server, &xrgb(&pixels, (4, 4)), INTEL_X_TILED);

    assert_eq!(client.protocol_error(), Some(Error::InvalidWlBuffer as u32));
}
