use std::cell::Cell;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTime {
    pub presentation: Instant,
    pub interval: Duration,
}

impl FrameTime {
    pub fn immediate(now: Instant) -> Self {
        Self {
            presentation: now,
            interval: Duration::ZERO,
        }
    }

    pub fn on_screen(&self) -> Instant {
        self.presentation
            .checked_sub(self.interval)
            .unwrap_or(self.presentation)
    }
}

thread_local! {
    static CURRENT: Cell<Option<FrameTime>> = const { Cell::new(None) };
    static WAKE: Cell<Option<Instant>> = const { Cell::new(None) };
}

pub fn set_frame_time(time: FrameTime) {
    CURRENT.set(Some(time));
}

pub fn frame_time() -> FrameTime {
    CURRENT
        .get()
        .unwrap_or_else(|| FrameTime::immediate(Instant::now()))
}

pub fn request_wake(at: Instant) {
    WAKE.set(Some(WAKE.get().map_or(at, |wake| wake.min(at))));
}

pub fn take_wake() -> Option<Instant> {
    WAKE.take()
}
