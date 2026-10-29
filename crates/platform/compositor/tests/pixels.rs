#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use smithay::utils::Size;
use unbound_compositor::{BACKGROUND, FrameStats, Headless, Renderer, Snapshot};
use unbound_test_wayland::{BLUE, GREEN, RED, ShmImage, TestClient};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (256, 192);

fn server() -> Headless {
    Headless::new(Size::from(OUTPUT)).expect("headless compositor")
}

fn render(renderer: &mut Renderer, server: &mut Headless) -> (FrameStats, Snapshot) {
    let stats = renderer.render(server).expect("rendered frame");
    (stats, renderer.snapshot(server).expect("read back frame"))
}

fn opaque_rgba(bgra: [u8; 4]) -> [u8; 4] {
    [bgra[2], bgra[1], bgra[0], u8::MAX]
}

fn pixel(snapshot: &Snapshot, x: i32, y: i32) -> [u8; 4] {
    let at = usize::try_from((y * snapshot.size.w + x) * 4).expect("pixel offset");
    snapshot.rgba[at..at + 4].try_into().expect("four channels")
}

fn background() -> [u8; 4] {
    [BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], u8::MAX]
}

fn solid(color: [u8; 4], side: usize) -> Vec<u8> {
    color.repeat(side * side)
}

fn image(bytes: &[u8], side: i32, format: Format) -> ShmImage<'_> {
    ShmImage {
        width: side,
        height: side,
        stride: side * 4,
        format,
        bytes,
    }
}

#[test]
fn xrgb_buffer_ignores_the_alpha_byte() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    let transparent_red = [RED[0], RED[1], RED[2], 0x00];
    let pixels = solid(transparent_red, 32);
    client.show_window(&mut server, &image(&pixels, 32, Format::Xrgb8888));
    let mut renderer = Renderer::new().expect("Vulkan renderer");

    let (_, snapshot) = render(&mut renderer, &mut server);

    let location = server.windows()[0].location();
    assert_eq!(
        pixel(&snapshot, location.x + 16, location.y + 16),
        opaque_rgba(RED)
    );
}

#[test]
fn row_padding_beyond_the_width_is_not_drawn() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    let (width, height, stride) = (16usize, 16usize, 16 * 4 + 12);
    let padded = quartered(width, stride, [[RED, GREEN], [BLUE, RED]]);
    client.show_window(
        &mut server,
        &ShmImage {
            width: width as i32,
            height: height as i32,
            stride: stride as i32,
            format: Format::Xrgb8888,
            bytes: &padded,
        },
    );
    let mut renderer = Renderer::new().expect("Vulkan renderer");

    let (_, snapshot) = render(&mut renderer, &mut server);

    let origin = server.windows()[0].location();
    let drawn: Vec<[u8; 4]> = [(7, 7), (8, 7), (7, 8), (8, 8)]
        .iter()
        .map(|(x, y)| pixel(&snapshot, origin.x + x, origin.y + y))
        .collect();
    let expected: Vec<[u8; 4]> = [RED, GREEN, BLUE, RED].map(opaque_rgba).to_vec();
    assert_eq!(drawn, expected);
    assert_ne!(pixel(&snapshot, origin.x + 16, origin.y + 8), [0xff; 4]);
}

fn quartered(side: usize, stride: usize, colors: [[[u8; 4]; 2]; 2]) -> Vec<u8> {
    let mut padded = vec![0xffu8; stride * side];
    for row in 0..side {
        for column in 0..side {
            let at = row * stride + column * 4;
            padded[at..at + 4].copy_from_slice(&colors[row * 2 / side][column * 2 / side]);
        }
    }
    padded
}

#[test]
fn the_first_frame_paints_the_whole_output() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("Vulkan renderer");

    let (stats, snapshot) = render(&mut renderer, &mut server);

    assert_eq!(stats.repainted_pixels, (OUTPUT.0 * OUTPUT.1) as u64);
    assert!(
        snapshot
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == background())
    );
}

#[test]
fn only_the_damaged_part_of_a_new_buffer_is_uploaded_and_repainted() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    let red = solid(RED, 64);
    client.show_window(&mut server, &image(&red, 64, Format::Xrgb8888));
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    render(&mut renderer, &mut server);

    let blue = solid(BLUE, 64);
    client.commit_buffer(0, &image(&blue, 64, Format::Xrgb8888), (8, 8, 16, 16));
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
        pixel(&snapshot, origin.x + 23, origin.y + 23),
        opaque_rgba(BLUE)
    );
    assert_eq!(
        pixel(&snapshot, origin.x + 7, origin.y + 8),
        opaque_rgba(RED)
    );
    assert_eq!(
        pixel(&snapshot, origin.x + 24, origin.y + 23),
        opaque_rgba(RED)
    );
}

#[test]
fn a_commit_without_a_new_buffer_repaints_nothing() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    let red = solid(RED, 8);
    client.show_window(&mut server, &image(&red, 8, Format::Xrgb8888));
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    render(&mut renderer, &mut server);

    client.request_frame(&mut server);
    let (stats, _) = render(&mut renderer, &mut server);

    assert_eq!(stats, FrameStats::default());
}

#[test]
fn closing_a_window_repaints_only_the_area_it_covered() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    let red = solid(RED, 32);
    client.show_window(&mut server, &image(&red, 32, Format::Xrgb8888));
    let origin = server.windows()[0].location();
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    render(&mut renderer, &mut server);

    client.close_window(&mut server, 0);
    let (stats, snapshot) = render(&mut renderer, &mut server);

    let active_shadow_reach = 3 * 14;
    let shadow_side = 32 + 2 * active_shadow_reach;
    assert_eq!(stats.repainted_pixels, shadow_side * shadow_side);
    assert_eq!(pixel(&snapshot, origin.x, origin.y), background());
    assert_eq!(pixel(&snapshot, origin.x + 31, origin.y + 31), background());
}

#[test]
fn premultiplied_argb_is_blended_over_what_lies_below() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    let half_red = [0x00, 0x00, 0x80, 0x80];
    let pixels = solid(half_red, 4);
    client.show_window(&mut server, &image(&pixels, 4, Format::Argb8888));
    let mut renderer = Renderer::new().expect("Vulkan renderer");

    let (_, snapshot) = render(&mut renderer, &mut server);

    let location = server.windows()[0].location();
    let below_after_half_alpha = |channel: u8| (u32::from(channel) * 0x7f / 0xff) as u8;
    let [r, g, b, _] = background();
    let expected = [
        0x80 + below_after_half_alpha(r),
        below_after_half_alpha(g),
        below_after_half_alpha(b),
        0xff,
    ];
    let actual = pixel(&snapshot, location.x + 2, location.y + 2);
    let close = actual
        .iter()
        .zip(expected)
        .all(|(actual, expected)| actual.abs_diff(expected) <= 1);
    assert!(close, "expected {expected:?}, rendered {actual:?}");
}

#[test]
fn an_opaque_window_hides_the_window_below_it() {
    let mut alone = server();
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    let red = solid(RED, 16);
    let green = solid(GREEN, 96);
    let mut lone_client = TestClient::connect(&mut alone);
    lone_client.show_window(&mut alone, &image(&green, 96, Format::Xrgb8888));
    client.show_window(&mut server, &image(&red, 16, Format::Xrgb8888));
    client.show_window(&mut server, &image(&green, 96, Format::Xrgb8888));
    let mut renderer = Renderer::new().expect("Vulkan renderer");

    let (stats, snapshot) = render(&mut renderer, &mut server);
    let mut lone_renderer = Renderer::new().expect("Vulkan renderer");
    let (unhidden, _) = render(&mut lone_renderer, &mut alone);

    assert_eq!(stats.shaded_pixels, unhidden.shaded_pixels);
    let origin = server.windows()[1].location();
    assert_eq!(
        pixel(&snapshot, origin.x + 48, origin.y + 48),
        opaque_rgba(GREEN)
    );
}

#[test]
fn damage_inside_an_opaque_window_shades_only_that_window() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    let red = solid(RED, 32);
    let green = solid(GREEN, 32);
    client.show_window(&mut server, &image(&red, 32, Format::Xrgb8888));
    client.show_window(&mut server, &image(&green, 32, Format::Xrgb8888));
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    render(&mut renderer, &mut server);

    let blue = solid(BLUE, 32);
    client.commit_buffer(1, &image(&blue, 32, Format::Xrgb8888), (8, 8, 16, 16));
    client.exchange(&mut server);
    let (stats, snapshot) = render(&mut renderer, &mut server);

    assert_eq!(stats.shaded_pixels, 16 * 16);
    let origin = server.windows()[1].location();
    assert_eq!(
        pixel(&snapshot, origin.x + 8, origin.y + 8),
        opaque_rgba(BLUE)
    );
}

#[test]
fn damage_far_beyond_the_buffer_uploads_and_repaints_only_the_buffer() {
    let mut server = server();
    let mut client = TestClient::connect(&mut server);
    let red = solid(RED, 16);
    client.show_window(&mut server, &image(&red, 16, Format::Xrgb8888));
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    render(&mut renderer, &mut server);

    let blue = solid(BLUE, 16);
    client.commit_buffer(
        0,
        &image(&blue, 16, Format::Xrgb8888),
        (0, 0, i32::MAX, i32::MAX),
    );
    client.exchange(&mut server);
    let (stats, snapshot) = render(&mut renderer, &mut server);

    assert_eq!(stats.uploaded_bytes, 16 * 16 * 4);
    assert_eq!(stats.repainted_pixels, 16 * 16);
    let origin = server.windows()[0].location();
    assert_eq!(
        pixel(&snapshot, origin.x + 8, origin.y + 8),
        opaque_rgba(BLUE)
    );
}
