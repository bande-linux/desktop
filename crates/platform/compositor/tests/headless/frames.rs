use std::io;
use std::time::Duration;

use bande_compositor::{
    Compositor, FrameClock, FrameOutput, FrameScheduler, Poll, REFRESH, RenderError, Renderer,
    SAFETY_MARGIN, Submission, Vblank,
};
use bande_test_wayland::{Presented, RED, ShmImage, TestClient};
use smithay::utils::Size;
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
    server: Compositor,
    client: TestClient,
    renderer: Renderer,
    scheduler: FrameScheduler,
    pixels: Vec<u8>,
}

impl Session {
    fn with_a_window() -> (Self, Vblank) {
        let mut server = Compositor::new(Size::from((256, 192))).expect("headless compositor");
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

struct Away;

impl FrameOutput for Away {
    type Error = RenderError;

    fn submit(&mut self, _: &mut Compositor, _: &str) -> Result<Submission, RenderError> {
        Ok(Submission::Paused)
    }
}

#[test]
fn an_output_away_from_the_session_keeps_clients_from_drawing_until_it_resumes() {
    let (mut session, last) = Session::with_a_window();
    let frames_before = session.client.frames_done();
    let mut now = last.time;

    for _ in 0..3 {
        session.commit_with_feedback();
        session.client.request_frame(&mut session.server);
        let poll = loop {
            let poll = session
                .scheduler
                .poll(|| now, &mut session.server, &mut Away)
                .expect("frame");
            match poll.wake_at {
                Some(wake_at) => now = wake_at,
                None => break poll,
            }
        };
        session.client.exchange(&mut session.server);
        assert_eq!(poll.presented, None);
        now += REFRESH;
    }
    assert_eq!(session.client.frames_done(), frames_before);
    assert!(session.client.presented().is_empty());

    session.scheduler.resume();
    session.run_from(now);
    assert!(session.client.frames_done() > frames_before);
}

fn milliseconds(count: u64) -> Duration {
    Duration::from_millis(count)
}

#[test]
fn a_slow_frame_moves_the_deadline_at_once_and_is_forgotten_later() {
    let mut clock = FrameClock::new(Duration::ZERO, REFRESH);
    let next = clock.vblank_after(Duration::ZERO);
    clock.record_render(Duration::ZERO, Duration::ZERO, milliseconds(2));
    clock.record_render(Duration::ZERO, Duration::ZERO, milliseconds(6));
    let after_the_slow_frame = clock.deadline(next);
    for _ in 0..100 {
        clock.record_render(Duration::ZERO, Duration::ZERO, milliseconds(2));
    }

    assert_eq!(
        after_the_slow_frame,
        next.time - milliseconds(6) - SAFETY_MARGIN
    );
    assert_eq!(
        clock.deadline(next),
        next.time - milliseconds(2) - SAFETY_MARGIN
    );
}

#[test]
fn a_render_slower_than_a_refresh_still_leaves_a_reachable_vblank() {
    let mut clock = FrameClock::new(Duration::ZERO, REFRESH);
    clock.record_render(Duration::ZERO, Duration::ZERO, REFRESH * 3);
    let now = REFRESH * 5 + milliseconds(1);

    let frame = clock.frame_for(now);

    assert_eq!(frame.sequence, 7);
    assert!(clock.deadline(frame) >= now);
}

fn late_wake_up(late_by: Duration) -> (Vblank, Option<Duration>, Vblank) {
    let (mut session, last) = Session::with_a_window();
    let mut clock = FrameClock::new(Duration::ZERO, REFRESH);
    clock.record_render(Duration::ZERO, milliseconds(2), milliseconds(4));
    session.scheduler = FrameScheduler::new(clock);
    let next = session.scheduler.clock().vblank_after(last.time);

    session.commit_with_feedback();
    let waiting = session
        .scheduler
        .poll(|| last.time, &mut session.server, &mut session.renderer)
        .expect("frame");
    let deadline = next.time - milliseconds(4) - SAFETY_MARGIN;
    (next, waiting.wake_at, session.run_from(deadline + late_by))
}

#[test]
fn a_wake_up_late_with_time_left_to_render_still_makes_its_vblank() {
    let (next, wake_at, presented) = late_wake_up(milliseconds(2));

    assert_eq!(wake_at, Some(next.time - milliseconds(4) - SAFETY_MARGIN));
    assert_eq!(presented.sequence, next.sequence);
}

#[test]
fn a_wake_up_too_late_to_render_in_time_draws_at_once_for_the_next_vblank() {
    let (next, _, presented) = late_wake_up(milliseconds(3) + MICROSECOND);

    assert_eq!(presented.sequence, next.sequence + 1);
}

struct Display {
    failing: bool,
    submitted: usize,
}

impl FrameOutput for Display {
    type Error = io::Error;

    fn submit(&mut self, _: &mut Compositor, _: &str) -> io::Result<Submission> {
        if self.failing {
            return Err(io::Error::other("the connector is gone"));
        }
        self.submitted += 1;
        Ok(Submission::AwaitingFlip)
    }
}

fn poll_display(session: &mut Session, display: &mut Display, now: Duration) -> io::Result<Poll> {
    session.scheduler.poll(|| now, &mut session.server, display)
}

fn submit_through(
    scheduler: &mut FrameScheduler,
    server: &mut Compositor,
    display: &mut Display,
    mut now: Duration,
) {
    let before = display.submitted;
    loop {
        let poll = scheduler.poll(|| now, server, display).expect("frame");
        if display.submitted > before {
            return;
        }
        now = poll.wake_at.expect("a frame is due");
    }
}

fn submit_through_session(session: &mut Session, display: &mut Display, now: Duration) {
    submit_through(&mut session.scheduler, &mut session.server, display, now);
}

#[test]
fn a_flip_lost_in_sleep_stops_holding_back_frames_once_awake() {
    let (mut session, last) = Session::with_a_window();
    let mut display = Display {
        failing: false,
        submitted: 0,
    };
    session.commit_with_feedback();
    submit_through_session(&mut session, &mut display, last.time);

    session.commit_with_feedback();
    let later = last.time + REFRESH * 10;
    assert_eq!(
        poll_display(&mut session, &mut display, later).expect("frame"),
        Poll::default()
    );

    session.scheduler.resume();
    submit_through_session(&mut session, &mut display, later);
    assert_eq!(display.submitted, 2);
}

#[test]
fn a_frame_the_display_refuses_is_dropped_instead_of_retried_every_refresh() {
    let (mut session, last) = Session::with_a_window();
    let mut display = Display {
        failing: true,
        submitted: 0,
    };
    session.commit_with_feedback();
    let mut now = last.time;
    while let Ok(poll) = poll_display(&mut session, &mut display, now) {
        now = poll.wake_at.expect("a frame is due");
    }

    let later = now + REFRESH * 3;
    assert_eq!(
        poll_display(&mut session, &mut display, later).expect("idle"),
        Poll::default()
    );

    display.failing = false;
    session.commit_with_feedback();
    submit_through_session(&mut session, &mut display, later);
    assert_eq!(display.submitted, 1);
}

#[test]
fn a_flip_lost_while_the_session_was_paused_does_not_stop_frames_after_it_resumes() {
    let mut server = Compositor::new(Size::from((64, 64))).expect("headless compositor");
    let mut display = Display {
        failing: false,
        submitted: 0,
    };
    let mut scheduler = FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH));
    scheduler.resume();
    submit_through(&mut scheduler, &mut server, &mut display, Duration::ZERO);

    let waiting = scheduler
        .poll(|| Duration::ZERO, &mut server, &mut display)
        .expect("frame");
    assert_eq!(waiting.wake_at, None);

    scheduler.resume();
    submit_through(&mut scheduler, &mut server, &mut display, Duration::ZERO);
    assert_eq!(display.submitted, 2);
}
