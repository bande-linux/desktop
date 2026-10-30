#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use unbound_compositor::Renderer;
use unbound_test_wayland::{BLUE, GREEN, HALF_WHITE, Memory, RED, ShmImage, quadrants};
use wayland_client::protocol::wl_shm::Format;

use crate::card::Scanout;
use crate::session::Session;

const SETTLE_FRAMES: usize = 300;

fn rgb(bgra: [u8; 4]) -> [u8; 3] {
    [bgra[2], bgra[1], bgra[0]]
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

fn present_until(
    session: &mut Session,
    what: &str,
    done: impl Fn(&Session, bool) -> bool,
) -> Scanout {
    for _ in 0..SETTLE_FRAMES {
        let damaged = session.present_damage();
        if done(session, damaged) {
            return session.scanout();
        }
    }
    panic!("no {what} within {SETTLE_FRAMES} frames");
}

fn composed_offscreen(session: &mut Session) -> Vec<[u8; 3]> {
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    renderer
        .render(&mut session.shared.server)
        .expect("offscreen frame");
    renderer
        .snapshot(&session.shared.server)
        .expect("read back the offscreen frame")
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&[r, g, b, _]| [r, g, b])
        .collect()
}

fn show_fullscreen(session: &mut Session, image: &ShmImage) {
    session
        .client
        .show_window_in(&mut session.shared.server, image, Memory::Udmabuf);
    session
        .client
        .set_fullscreen(&mut session.shared.server, 0, true);
    session
        .client
        .commit_buffer_in(0, image, (0, 0, image.width, image.height), Memory::Udmabuf);
    session.client.exchange(&mut session.shared.server);
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

    let scanout = present_until(&mut session, "settled screen", |_, damaged| !damaged);
    scanout.save("composed-window");

    assert!(
        !session
            .output
            .scanned_out_directly(&session.shared.server.main_output())
    );
    assert_eq!((scanout.width, scanout.height), (width, height));
    let composed = composed_offscreen(&mut session);
    assert_matches(&scanout, |x, y| composed[y * width + x]);
}

#[test]
#[ignore = "needs the ci/vm guest"]
fn a_fullscreen_opaque_dmabuf_window_is_scanned_out_directly() {
    let mut session = Session::open();
    let (width, height) = session.output_size();
    let pixels = quadrants(width, height, width * 4);
    show_fullscreen(
        &mut session,
        &ShmImage {
            width: width as i32,
            height: height as i32,
            stride: (width * 4) as i32,
            format: Format::Xrgb8888,
            bytes: &pixels,
        },
    );

    let scanout = present_until(&mut session, "direct scanout", |session, _| {
        session
            .output
            .scanned_out_directly(&session.shared.server.main_output())
    });
    scanout.save("direct-scanout");

    assert_matches(&scanout, |x, y| rgb(quadrant_color(x, y, width, height)));
}

#[test]
#[ignore = "needs the ci/vm guest"]
fn an_opaque_dmabuf_window_on_an_overlay_plane_shows_its_pixels() {
    let mut session = Session::open();
    let width = session.output_size().0;
    let (window_width, window_height) = (64, 64);
    let pixels = RED.repeat(window_width * window_height);
    show_fullscreen(
        &mut session,
        &ShmImage {
            width: window_width as i32,
            height: window_height as i32,
            stride: (window_width * 4) as i32,
            format: Format::Xrgb8888,
            bytes: &pixels,
        },
    );

    let scanout = present_until(&mut session, "overlay plane", |session, _| {
        session.card.overlay_planes_in_use() == 1
    });
    scanout.save("overlay-window");

    let composed = composed_offscreen(&mut session);
    assert!(composed.contains(&rgb(RED)), "the window is composed");
    assert_matches(&scanout, |x, y| composed[y * width + x]);
}
