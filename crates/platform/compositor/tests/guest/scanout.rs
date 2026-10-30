#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use unbound_compositor::BACKGROUND;
use unbound_test_wayland::{BLUE, GREEN, HALF_WHITE, Memory, RED, ShmImage, quadrants};
use wayland_client::protocol::wl_shm::Format;

use crate::card::Scanout;
use crate::session::Session;

fn rgb(bgra: [u8; 4]) -> [u8; 3] {
    [bgra[2], bgra[1], bgra[0]]
}

fn over_background(bgra: [u8; 4]) -> [u8; 3] {
    let alpha = u32::from(bgra[3]);
    let [r, g, b] = rgb(bgra);
    [r, g, b]
        .iter()
        .zip(BACKGROUND)
        .map(|(&source, below)| {
            (u32::from(source) + u32::from(below) * (255 - alpha) / 255).min(255) as u8
        })
        .collect::<Vec<_>>()
        .try_into()
        .expect("three channels")
}

fn quadrant_color(x: usize, y: usize, width: usize, height: usize) -> [u8; 4] {
    match (x < width / 2, y < height / 2) {
        (true, true) => RED,
        (false, true) => GREEN,
        (true, false) => BLUE,
        (false, false) => HALF_WHITE,
    }
}

fn assert_matches(scanout: &Scanout, expected: impl Fn(usize, usize) -> [u8; 3]) {
    let mismatches: Vec<(usize, usize, [u8; 3], [u8; 3])> = (0..scanout.height)
        .flat_map(|y| (0..scanout.width).map(move |x| (x, y)))
        .filter_map(|(x, y)| {
            let actual = scanout.rgb[y * scanout.width + x];
            let wanted = expected(x, y);
            let close = actual
                .iter()
                .zip(wanted)
                .all(|(actual, wanted)| actual.abs_diff(wanted) <= 1);
            (!close).then_some((x, y, actual, wanted))
        })
        .collect();
    assert!(
        mismatches.is_empty(),
        "{} pixels differ, first at {:?}",
        mismatches.len(),
        mismatches.first()
    );
}

#[test]
#[ignore = "needs the ci/vm guest"]
fn a_composed_window_is_scanned_out() {
    let mut session = Session::open();
    let (width, height) = session.output_size();
    let (window_width, window_height) = (96, 64);
    let pixels = quadrants(window_width, window_height, window_width * 4);
    session.client.show_window(
        &mut session.shared.server,
        &ShmImage {
            width: window_width as i32,
            height: window_height as i32,
            stride: (window_width * 4) as i32,
            format: Format::Argb8888,
            bytes: &pixels,
        },
    );

    let scanout = session.present();
    scanout.save("composed-window");

    assert!(
        !session
            .output
            .scanned_out_directly(&session.shared.server.main_output())
    );
    assert_eq!((scanout.width, scanout.height), (width, height));
    let left = (width - window_width) / 2;
    let top = (height - window_height) / 2;
    assert_matches(&scanout, |x, y| {
        let inside =
            (left..left + window_width).contains(&x) && (top..top + window_height).contains(&y);
        if inside {
            over_background(quadrant_color(
                x - left,
                y - top,
                window_width,
                window_height,
            ))
        } else {
            BACKGROUND
        }
    });
}

#[test]
#[ignore = "needs the ci/vm guest"]
fn a_fullscreen_opaque_dmabuf_window_is_scanned_out_directly() {
    let mut session = Session::open();
    let (width, height) = session.output_size();
    let pixels = quadrants(width, height, width * 4);
    session.client.show_window_in(
        &mut session.shared.server,
        &ShmImage {
            width: width as i32,
            height: height as i32,
            stride: (width * 4) as i32,
            format: Format::Xrgb8888,
            bytes: &pixels,
        },
        Memory::Udmabuf,
    );

    let scanout = session.present();
    scanout.save("direct-scanout");

    assert!(
        session
            .output
            .scanned_out_directly(&session.shared.server.main_output())
    );
    assert_matches(&scanout, |x, y| rgb(quadrant_color(x, y, width, height)));
}

#[test]
#[ignore = "needs the ci/vm guest"]
fn an_opaque_dmabuf_window_on_an_overlay_plane_shows_its_pixels() {
    let mut session = Session::open();
    let (width, height) = session.output_size();
    let (window_width, window_height) = (64, 64);
    let pixels = RED.repeat(window_width * window_height);
    session.client.show_window_in(
        &mut session.shared.server,
        &ShmImage {
            width: window_width as i32,
            height: window_height as i32,
            stride: (window_width * 4) as i32,
            format: Format::Xrgb8888,
            bytes: &pixels,
        },
        Memory::Udmabuf,
    );

    let scanout = session.present();
    scanout.save("overlay-window");

    assert_eq!(session.card.overlay_planes_in_use(), 1);
    let left = (width - window_width) / 2;
    let top = (height - window_height) / 2;
    assert_matches(&scanout, |x, y| {
        let inside =
            (left..left + window_width).contains(&x) && (top..top + window_height).contains(&y);
        if inside { rgb(RED) } else { BACKGROUND }
    });
}
