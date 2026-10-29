#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use smithay::utils::Size;
use unbound_compositor::{
    FrameClock, FrameScheduler, Headless, REFRESH, Renderer, SAFETY_MARGIN, Vblank,
};
use unbound_test_wayland::{Presented, RED, ShmImage, TestClient};
use wayland_client::protocol::wl_shm::Format;

const SIDE: i32 = 16;
const MICROSECOND: Duration = Duration::from_micros(1);

fn image(pixels: &[u8]) -> ShmImage<'_> {
    ShmImage {
        width: SIDE,
        height: SIDE,
        stride: SIDE * 4,
        format: Format::Xrgb8888,
        bytes: pixels,
    }
}

fn vblank_time(sequence: u64) -> Duration {
    REFRESH * u32::try_from(sequence).expect("sequence")
}

struct Session {
    server: Headless,
    client: TestClient,
    renderer: Renderer,
    scheduler: FrameScheduler,
    pixels: Vec<u8>,
}

impl Session {
    fn with_a_window() -> (Self, Vblank) {
        let mut server = Headless::new(Size::from((256, 192))).expect("headless compositor");
        let mut client = TestClient::connect(&mut server);
        let pixels = RED.repeat((SIDE * SIDE) as usize);
        client.show_window(&mut server, &image(&pixels));
        let mut session = Self {
            server,
            client,
            renderer: Renderer::new().expect("Vulkan renderer"),
            scheduler: FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH)),
            pixels,
        };
        let mut settled = session.run_from(Duration::ZERO);
        while session.server.is_animating() {
            settled = session.run_from(settled.time);
        }
        (session, settled)
    }

    fn deadline_of_next_vblank(&self, after: Vblank) -> (Vblank, Duration) {
        let clock = self.scheduler.clock();
        let next = clock.vblank_after(after.time);
        (next, clock.deadline(next))
    }

    fn commit_with_feedback(&mut self) {
        self.client.request_presentation(0);
        self.client
            .commit_buffer(0, &image(&self.pixels), (0, 0, SIDE, SIDE));
        self.server.dispatch().expect("server dispatch");
    }

    fn run_from(&mut self, mut now: Duration) -> Vblank {
        loop {
            let poll = self
                .scheduler
                .poll(|| now, &mut self.server, &mut self.renderer)
                .expect("frame");
            if let Some(vblank) = poll.presented {
                self.client.exchange(&mut self.server);
                return vblank;
            }
            now = poll.wake_at.expect("a frame is due");
        }
    }
}

#[test]
fn a_commit_before_the_deadline_is_presented_at_the_next_vblank() {
    let (mut session, last) = Session::with_a_window();
    let (next, deadline) = session.deadline_of_next_vblank(last);
    assert_eq!(deadline, next.time - SAFETY_MARGIN);

    session.commit_with_feedback();
    let presented = session.run_from(deadline);

    assert_eq!(presented.sequence, last.sequence + 1);
    assert_eq!(
        session.client.presented(),
        [Presented {
            time: vblank_time(next.sequence),
            refresh: REFRESH,
            sequence: next.sequence,
            vsync: true,
        }]
    );
}

#[test]
fn a_commit_after_the_deadline_is_presented_one_vblank_later() {
    let (mut session, last) = Session::with_a_window();
    let (next, deadline) = session.deadline_of_next_vblank(last);

    session.commit_with_feedback();
    let presented = session.run_from(deadline + MICROSECOND);

    assert_eq!(presented.sequence, next.sequence + 1);
    assert_eq!(
        session.client.presented()[0].time,
        vblank_time(next.sequence + 1)
    );
}

#[test]
fn a_commit_superseded_before_the_deadline_is_discarded() {
    let (mut session, last) = Session::with_a_window();

    session.commit_with_feedback();
    session.commit_with_feedback();
    session.run_from(last.time);

    assert_eq!(session.client.discarded(), 1);
    assert_eq!(session.client.presented().len(), 1);
}

#[test]
fn nothing_is_composed_while_no_client_commits() {
    let (mut session, last) = Session::with_a_window();

    let poll = session
        .scheduler
        .poll(|| last.time, &mut session.server, &mut session.renderer)
        .expect("frame");

    assert_eq!(poll.wake_at, None);
}
