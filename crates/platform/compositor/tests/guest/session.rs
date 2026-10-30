#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use smithay::backend::drm::{DrmEvent, DrmEventMetadata};
use smithay::backend::session::libseat::LibSeatSessionNotifier;
use smithay::reexports::calloop::EventLoop;
use unbound_compositor::{DrmNotifiers, DrmOutput, FrameOutput, Headless, Submission};
use unbound_test_wayland::TestClient;

use crate::card::{Card, Scanout, card_path};

const FLIP_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Shared {
    pub server: Headless,
    flip: Option<Option<DrmEventMetadata>>,
}

pub struct Session {
    pub card: Card,
    pub output: DrmOutput,
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
            DrmOutput::open(Some(&path)).expect("the compositor takes the card through libseat");
        let mut server = Headless::with_output(output.output_config()).expect("compositor state");
        let client = TestClient::connect(&mut server);
        let events = EventLoop::try_new().expect("event loop");
        events
            .handle()
            .insert_source(device, |event, metadata, shared: &mut Shared| {
                if let DrmEvent::VBlank(_) = event {
                    shared.flip = Some(*metadata);
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
        let main = self.shared.server.main_output();
        let submission = self
            .output
            .submit(&mut self.shared.server, &main)
            .expect("frame");
        assert_eq!(submission, Submission::AwaitingFlip);
        self.shared.flip = None;
        self.dispatch_until("a page flip", |session| session.shared.flip.is_some());
        let metadata = self.shared.flip.take().flatten();
        self.output.flipped(metadata).expect("frame completion");
        self.card.scanout()
    }
}
