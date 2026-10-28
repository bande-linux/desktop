use std::time::Duration;

use smithay::backend::drm::{DrmEvent, DrmEventMetadata};
use smithay::reexports::calloop::EventLoop;
use unbound_compositor::{BACKGROUND, DrmNotifiers, DrmOutput, FrameOutput, Headless, Submission};
use unbound_test_wayland::{BLUE, GREEN, HALF_WHITE, Memory, RED, ShmImage, TestClient, quadrants};
use wayland_client::protocol::wl_shm::Format;

use crate::card::{Card, Scanout, card_path};

const FLIP_TIMEOUT: Duration = Duration::from_secs(5);

type Flip = Option<Option<DrmEventMetadata>>;

struct Session {
    card: Card,
    output: DrmOutput,
    server: Headless,
    client: TestClient,
    events: EventLoop<'static, Flip>,
    _notifiers: smithay::backend::session::libseat::LibSeatSessionNotifier,
}

impl Session {
    fn on_vkms() -> Self {
        let path = card_path("vkms");
        let (output, DrmNotifiers { session, device }) =
            DrmOutput::open(Some(&path)).expect("the compositor takes vkms through libseat");
        let mut server = Headless::with_output(output.output_config()).expect("compositor state");
        let client = TestClient::connect(&mut server);
        let events = EventLoop::try_new().expect("event loop");
        events
            .handle()
            .insert_source(device, |event, metadata, flip: &mut Flip| {
                if let DrmEvent::VBlank(_) = event {
                    *flip = Some(*metadata);
                }
            })
            .expect("DRM events");
        let mut session = Self {
            card: Card::open(&path).expect("second vkms fd for readback"),
            output,
            server,
            client,
            events,
            _notifiers: session,
        };
        session.present();
        session
    }

    fn output_size(&self) -> (usize, usize) {
        let size = self.output.output_config().size;
        (size.w as usize, size.h as usize)
    }

    fn present(&mut self) -> Scanout {
        let submission = self.output.submit(&mut self.server).expect("frame");
        assert_eq!(submission, Submission::AwaitingFlip);
        let mut flip = None;
        let started = std::time::Instant::now();
        while flip.is_none() && started.elapsed() < FLIP_TIMEOUT {
            self.events
                .dispatch(Some(Duration::from_millis(100)), &mut flip)
                .expect("dispatch DRM events");
        }
        let metadata = flip.expect("a page flip after the commit");
        self.output.flipped(metadata).expect("frame completion");
        self.card.scanout()
    }
}

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
fn a_composed_window_is_scanned_out_on_vkms() {
    let mut session = Session::on_vkms();
    let (width, height) = session.output_size();
    let (window_width, window_height) = (96, 64);
    let pixels = quadrants(window_width, window_height, window_width * 4);
    session.client.show_window(
        &mut session.server,
        &ShmImage {
            width: window_width as i32,
            height: window_height as i32,
            stride: (window_width * 4) as i32,
            format: Format::Argb8888,
            bytes: &pixels,
        },
    );

    let scanout = session.present();

    assert!(!session.output.scanned_out_directly());
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
    let mut session = Session::on_vkms();
    let (width, height) = session.output_size();
    let pixels = quadrants(width, height, width * 4);
    session.client.show_window_in(
        &mut session.server,
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

    assert!(session.output.scanned_out_directly());
    assert_matches(&scanout, |x, y| rgb(quadrant_color(x, y, width, height)));
}
