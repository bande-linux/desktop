use smithay::utils::Size;
use unbound_compositor::Headless;
use unbound_test_wayland::{DecorationMode, RED, ShmImage, TestClient};
use wayland_client::protocol::wl_shm::Format;

fn decorated(requested: Option<DecorationMode>) -> TestClient {
    let mut server = Headless::new(Size::from((256, 192))).expect("headless compositor");
    let mut client = TestClient::connect(&mut server);
    let pixels = RED.repeat(32 * 32);
    client.show_decorated_window(
        &mut server,
        &ShmImage {
            width: 32,
            height: 32,
            stride: 32 * 4,
            format: Format::Xrgb8888,
            bytes: &pixels,
        },
        requested,
    );
    client.exchange(&mut server);
    assert_eq!(client.protocol_error(), None);
    client
}

#[test]
fn a_window_that_asks_for_server_side_decorations_is_told_to_draw_its_own() {
    let client = decorated(Some(DecorationMode::ServerSide));

    assert_eq!(
        client.decoration_modes().last(),
        Some(&DecorationMode::ClientSide)
    );
}

#[test]
fn a_window_that_leaves_the_choice_to_the_compositor_draws_its_own_decorations() {
    let client = decorated(None);

    assert_eq!(
        client.decoration_modes().last(),
        Some(&DecorationMode::ClientSide)
    );
}
