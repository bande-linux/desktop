use smithay::utils::Size;
use unbound_compositor::{FrameStats, Headless, Renderer, Snapshot};
use unbound_test_wayland::{BLUE, GREEN, LINEAR, Memory, RED, ShmImage, TestClient};
use wayland_client::protocol::wl_shm::Format;
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::Error;

const INTEL_X_TILED: u64 = 0x0100_0000_0000_0001;

fn server() -> Headless {
    Headless::new(Size::from((256, 192))).expect("headless compositor")
}

fn render(renderer: &mut Renderer, server: &mut Headless) -> (FrameStats, Snapshot) {
    let stats = renderer.render(server).expect("rendered frame");
    (stats, renderer.snapshot(server).expect("read back frame"))
}

fn pixel(snapshot: &Snapshot, x: i32, y: i32) -> [u8; 4] {
    let at = usize::try_from((y * snapshot.size.w + x) * 4).expect("pixel offset");
    snapshot.rgba[at..at + 4].try_into().expect("four channels")
}

fn opaque_rgba(bgra: [u8; 4]) -> [u8; 4] {
    [bgra[2], bgra[1], bgra[0], u8::MAX]
}

fn solid(color: [u8; 4], side: i32) -> ShmImage<'static> {
    let bytes = color.repeat(usize::try_from(side * side).expect("side"));
    ShmImage {
        width: side,
        height: side,
        stride: side * 4,
        format: Format::Xrgb8888,
        bytes: bytes.leak(),
    }
}

#[test]
fn a_linear_dmabuf_with_row_padding_is_drawn_without_it() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    let stride = 2 * 4 + 8;
    let mut padded = vec![0xffu8; stride * 2];
    for (row, colors) in [[RED, GREEN], [BLUE, RED]].iter().enumerate() {
        for (column, color) in colors.iter().enumerate() {
            let at = row * stride + column * 4;
            padded[at..at + 4].copy_from_slice(color);
        }
    }
    let image = ShmImage {
        width: 2,
        height: 2,
        stride: stride as i32,
        format: Format::Xrgb8888,
        bytes: &padded,
    };
    client.show_window_in(&mut server, &image, Memory::Dmabuf);
    let mut renderer = Renderer::new().expect("Vulkan renderer");

    let (_, snapshot) = render(&mut renderer, &mut server);

    let origin = server.windows()[0].location();
    let drawn: Vec<[u8; 4]> = [(0, 0), (1, 0), (0, 1), (1, 1)]
        .iter()
        .map(|(x, y)| pixel(&snapshot, origin.x + x, origin.y + y))
        .collect();
    assert_eq!(drawn, [RED, GREEN, BLUE, RED].map(opaque_rgba).to_vec());
}

#[test]
fn only_the_damaged_part_of_a_dmabuf_is_uploaded_and_repainted() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    client.show_window_in(&mut server, &solid(RED, 64), Memory::Dmabuf);
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    render(&mut renderer, &mut server);

    client.commit_buffer_in(0, &solid(BLUE, 64), (8, 8, 16, 16), Memory::Dmabuf);
    client.exchange(&mut server);
    let (stats, snapshot) = render(&mut renderer, &mut server);

    assert_eq!(
        stats,
        FrameStats {
            uploaded_bytes: 16 * 16 * 4,
            repainted_pixels: 16 * 16,
            shaded_pixels: 16 * 16,
        }
    );
    let origin = server.windows()[0].location();
    assert_eq!(
        pixel(&snapshot, origin.x + 8, origin.y + 8),
        opaque_rgba(BLUE)
    );
    assert_eq!(
        pixel(&snapshot, origin.x + 7, origin.y + 8),
        opaque_rgba(RED)
    );
}

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
fn a_dmabuf_with_a_modifier_that_was_not_offered_is_an_invalid_format() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);

    client.create_dmabuf(&mut server, &solid(RED, 4), INTEL_X_TILED);

    assert_eq!(client.protocol_error(), Some(Error::InvalidFormat as u32));
}
