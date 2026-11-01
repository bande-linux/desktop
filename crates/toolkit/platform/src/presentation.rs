use std::time::{Duration, Instant};

use bande_widgets::FrameTime;
use rustix::time::{ClockId, clock_gettime};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::presentation_time::client::wp_presentation::{self, WpPresentation};
use wayland_protocols::wp::presentation_time::client::wp_presentation_feedback::{
    self, WpPresentationFeedback,
};

use crate::window::State;

const CLOCK_MONOTONIC: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Presented {
    at: Instant,
    refresh: Duration,
    latency: Duration,
}

#[derive(Default)]
pub(crate) struct Presentation {
    pub(crate) manager: Option<WpPresentation>,
    monotonic: bool,
    last: Option<Presented>,
    in_flight: Option<Instant>,
}

impl Presentation {
    pub(crate) fn committing(
        &mut self,
        surface: &WlSurface,
        handle: &QueueHandle<State>,
        frame: FrameTime,
    ) {
        if let Some(manager) = self.manager.as_ref().filter(|_| self.monotonic) {
            manager.feedback(surface, handle, Instant::now());
        }
        self.in_flight = Some(frame.presentation);
    }

    pub(crate) fn frame_time(&self, now: Instant, frame_pending: bool) -> FrameTime {
        let Some(last) = self.last.filter(|last| !last.refresh.is_zero()) else {
            return FrameTime::immediate(now);
        };
        let earliest = now + last.latency.saturating_sub(last.refresh / 2);
        let mut presentation = vblank_at_or_after(last.at, last.refresh, earliest);
        if frame_pending && let Some(in_flight) = self.in_flight {
            presentation = presentation.max(in_flight + last.refresh);
        }
        FrameTime {
            presentation,
            interval: last.refresh,
        }
    }

    fn presented(&mut self, at: Instant, refresh: Duration, committed: Instant) {
        self.last = Some(Presented {
            at,
            refresh,
            latency: at.saturating_duration_since(committed),
        });
    }
}

fn vblank_at_or_after(anchor: Instant, refresh: Duration, earliest: Instant) -> Instant {
    let behind = earliest.saturating_duration_since(anchor).as_nanos();
    let periods = behind.div_ceil(refresh.as_nanos());
    let ahead = refresh.as_nanos().saturating_mul(periods);
    anchor + Duration::from_nanos(u64::try_from(ahead).unwrap_or(u64::MAX))
}

fn monotonic_instant(seconds: u64, nanoseconds: u32) -> Instant {
    let at =
        Duration::from_secs(seconds).saturating_add(Duration::from_nanos(u64::from(nanoseconds)));
    let now = Instant::now();
    let clock = clock_gettime(ClockId::Monotonic);
    let clock = Duration::new(
        u64::try_from(clock.tv_sec).unwrap_or_default(),
        u32::try_from(clock.tv_nsec).unwrap_or_default(),
    );
    match clock.checked_sub(at) {
        Some(age) => now.checked_sub(age).unwrap_or(now),
        None => now.checked_add(at - clock).unwrap_or(now),
    }
}

impl Dispatch<WpPresentation, ()> for State {
    fn event(
        state: &mut Self,
        _: &WpPresentation,
        event: wp_presentation::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_presentation::Event::ClockId { clk_id } = event {
            state.presentation.monotonic = clk_id == CLOCK_MONOTONIC;
        }
    }
}

impl Dispatch<WpPresentationFeedback, Instant> for State {
    fn event(
        state: &mut Self,
        _: &WpPresentationFeedback,
        event: wp_presentation_feedback::Event,
        committed: &Instant,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_presentation_feedback::Event::Presented {
            tv_sec_hi,
            tv_sec_lo,
            tv_nsec,
            refresh,
            ..
        } = event
        {
            let seconds = (u64::from(tv_sec_hi) << 32) | u64::from(tv_sec_lo);
            state.presentation.presented(
                monotonic_instant(seconds, tv_nsec),
                Duration::from_nanos(u64::from(refresh)),
                *committed,
            );
        }
    }
}
