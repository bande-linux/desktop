#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use smithay::backend::drm::{DrmEvent, DrmEventMetadata};
use smithay::backend::session::libseat::LibSeatSessionNotifier;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::drm::control::crtc;
use unbound_compositor::{DrmBackend, DrmNotifiers, FrameOutput, Headless, Submission};
use unbound_test_wayland::TestClient;

use crate::card::{Card, Scanout, card_path};

const FLIP_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Shared {
    pub server: Headless,
    flip: Option<(crtc::Handle, Option<DrmEventMetadata>)>,
}

pub struct Session {
    pub card: Card,
    pub output: DrmBackend,
    pub shared: Shared,
    pub client: TestClient,
    pub events: EventLoop<'static, Shared>,
    _notifiers: LibSeatSessionNotifier,
}

impl Session {
    pub fn open() -> Self {
        Self::on(&std::env::var("UNBOUND_TEST_DRIVER").unwrap_or_else(|_| "vkms".to_owned()))
    }

    pub fn on(driver: &str) -> Self {
        let path = card_path(driver);
        let (output, DrmNotifiers { session, device }) =
            DrmBackend::open(Some(&path)).expect("the compositor takes the card through libseat");
        let first = output
            .outputs()
            .into_iter()
            .next()
            .expect("a connected display");
        let mut server =
            Headless::on_device(first, Some(output.scanout_device())).expect("compositor state");
        let client = TestClient::connect(&mut server);
        let events = EventLoop::try_new().expect("event loop");
        events
            .handle()
            .insert_source(device, |event, metadata, shared: &mut Shared| {
                if let DrmEvent::VBlank(crtc) = event {
                    shared.flip = Some((crtc, *metadata));
                }
            })
            .expect("DRM events");
        let mut session = Self {
            card: Card::open(&path).expect("second vkms fd for readback"),
            output,
            shared: Shared { server, flip: None },
            client,
            events,
            _notifiers: session,
        };
        session.present();
        session
    }

    pub fn output_size(&self) -> (usize, usize) {
        let size = self.shared.server.output_size();
        (size.w as usize, size.h as usize)
    }

    pub fn dispatch_until(&mut self, what: &str, done: impl Fn(&mut Self) -> bool) {
        let started = Instant::now();
        while !done(self) {
            assert!(
                started.elapsed() < FLIP_TIMEOUT,
                "timed out waiting for {what}"
            );
            self.events
                .dispatch(Some(Duration::from_millis(20)), &mut self.shared)
                .expect("dispatch events");
            self.client.exchange(&mut self.shared.server);
        }
    }

    pub fn scanout(&self) -> Scanout {
        self.card.scanout()
    }

    pub fn present(&mut self) -> Scanout {
        assert!(self.present_damage(), "a frame with damage");
        self.card.scanout()
    }

    pub fn present_damage(&mut self) -> bool {
        let main = self.shared.server.main_output();
        let submission = self
            .output
            .submit(&mut self.shared.server, &main)
            .expect("frame");
        if submission != Submission::AwaitingFlip {
            return false;
        }
        self.shared.flip = None;
        self.dispatch_until("a page flip", |session| session.shared.flip.is_some());
        let (crtc, metadata) = self.shared.flip.take().expect("a page flip");
        self.output
            .flipped(crtc, metadata)
            .expect("frame completion");
        true
    }
}
