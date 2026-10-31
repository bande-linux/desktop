use std::path::Path;

use image::{RgbaImage, buffer::ConvertBuffer};
use image_compare::Algorithm;
use smithay::utils::Size;
use unbound_compositor::{Compositor, Renderer};
use unbound_test_wayland::{ShmImage, TestClient, quadrants};
use wayland_client::protocol::wl_shm::Format;

const MIN_SSIM: f64 = 0.995;
const MAX_CHANNEL_DELTA: u8 = 2;

#[test]
fn centered_window_matches_the_golden_image() {
    let mut server = Compositor::new(Size::from((256, 192))).expect("headless compositor");
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

    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let info = renderer.device_info();
    println!("device: {} ({})", info.name, info.driver);
    renderer.render(&mut server).expect("rendered frame");
    let snapshot = renderer.snapshot(&server).expect("read back frame");

    let actual = RgbaImage::from_raw(
        snapshot.size.w.unsigned_abs(),
        snapshot.size.h.unsigned_abs(),
        snapshot.rgba,
    )
    .expect("snapshot dimensions");
    let actual_path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("centered-quadrants.png");
    actual.save(&actual_path).expect("save rendered frame");

    let golden = image::open(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/centered-quadrants.png"),
    )
    .expect("golden image")
    .to_rgba8();
    assert_eq!(actual.dimensions(), golden.dimensions());

    let ssim = image_compare::rgb_similarity_structure(
        &Algorithm::MSSIMSimple,
        &actual.convert(),
        &golden.convert(),
    )
    .expect("SSIM")
    .score;
    let max_delta = actual
        .as_raw()
        .iter()
        .zip(golden.as_raw())
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    assert!(
        ssim >= MIN_SSIM && max_delta <= MAX_CHANNEL_DELTA,
        "SSIM {ssim:.5} (min {MIN_SSIM}), max channel delta {max_delta} (max {MAX_CHANNEL_DELTA}); rendered frame at {}",
        actual_path.display()
    );
}
