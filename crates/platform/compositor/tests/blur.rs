#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use smithay::utils::Size;
use unbound_compositor::{Headless, Renderer, Snapshot};
use unbound_test_wayland::{ShmImage, TestClient, roundtrip};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (256, 192);
const BELOW: (i32, i32) = (128, 96);
const GLASS: i32 = 32;
const ROW: i32 = 96;
const BLACK: [u8; 4] = [0, 0, 0, 0xff];
const WHITE: [u8; 4] = [0xff; 4];

fn below_pixels(white_from: i32) -> Vec<u8> {
    (0..BELOW.1)
        .flat_map(|_| (0..BELOW.0).flat_map(move |x| if x < white_from { BLACK } else { WHITE }))
        .collect()
}

fn image(bytes: &[u8], width: i32, height: i32, format: Format) -> ShmImage<'_> {
    ShmImage {
        width,
        height,
        stride: width * 4,
        format,
        bytes,
    }
}

fn row(snapshot: &Snapshot) -> Vec<u8> {
    (112..112 + GLASS)
        .map(|x| snapshot.rgba[((ROW * snapshot.size.w + x) * 4) as usize])
        .collect()
}

fn scene(blurred: bool) -> (Headless, Renderer, TestClient, TestClient) {
    let mut server = Headless::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut below = TestClient::connect(&mut server);
    let pixels = below_pixels(BELOW.0 / 2);
    below.show_window(
        &mut server,
        &image(&pixels, BELOW.0, BELOW.1, Format::Xrgb8888),
    );
    let mut glass = TestClient::connect(&mut server);
    let clear = vec![0u8; (GLASS * GLASS * 4) as usize];
    glass.show_window(&mut server, &image(&clear, GLASS, GLASS, Format::Argb8888));
    if blurred {
        glass.blur_behind(&mut server, 0, (0, 0, GLASS, GLASS));
    }
    roundtrip(&mut server, &mut [&mut below, &mut glass]);
    (server, Renderer::new().expect("renderer"), below, glass)
}

#[test]
fn a_surface_that_asks_for_blur_shows_what_lies_behind_it_softened() {
    let (mut sharp_server, mut sharp_renderer, _sharp_below, _sharp_glass) = scene(false);
    let (mut server, mut renderer, _below, _glass) = scene(true);

    sharp_renderer.render(&mut sharp_server).expect("frame");
    renderer.render(&mut server).expect("frame");
    let sharp = row(&sharp_renderer.snapshot(&sharp_server).expect("read back"));
    let soft = row(&renderer.snapshot(&server).expect("read back"));

    assert!(
        sharp.iter().all(|value| *value == 0 || *value == 0xff),
        "{sharp:?}"
    );
    assert!(soft.windows(2).all(|pair| pair[0] <= pair[1]), "{soft:?}");
    assert!(
        soft.iter()
            .filter(|value| **value > 8 && **value < 247)
            .count()
            >= 8,
        "{soft:?}"
    );
}

#[test]
fn a_change_behind_the_blur_but_outside_it_redraws_the_blur() {
    let (mut server, mut renderer, mut below, _glass) = scene(true);
    renderer.render(&mut server).expect("frame");
    let before = row(&renderer.snapshot(&server).expect("read back"))[2];

    let brighter = below_pixels(24);
    below.commit_buffer(
        0,
        &image(&brighter, BELOW.0, BELOW.1, Format::Xrgb8888),
        (24, 0, 20, BELOW.1),
    );
    below.exchange(&mut server);
    renderer.render(&mut server).expect("frame");
    let after = row(&renderer.snapshot(&server).expect("read back"))[2];

    assert!(after > before.saturating_add(20), "{before} then {after}");
}
