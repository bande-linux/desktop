use std::collections::VecDeque;
use std::time::Duration;

use unbound_render::RenderError;

use crate::render::Renderer;
use crate::state::Headless;

pub const SAFETY_MARGIN: Duration = Duration::from_millis(1);

const REMEMBERED_RENDERS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vblank {
    pub sequence: u64,
    pub time: Duration,
}

pub struct FrameClock {
    origin: Duration,
    refresh: Duration,
    render_times: VecDeque<Duration>,
}

impl FrameClock {
    pub fn new(origin: Duration, refresh: Duration) -> Self {
        Self {
            origin,
            refresh,
            render_times: VecDeque::with_capacity(REMEMBERED_RENDERS),
        }
    }

    pub fn refresh(&self) -> Duration {
        self.refresh
    }

    pub fn vblank_after(&self, now: Duration) -> Vblank {
        let elapsed = now.saturating_sub(self.origin);
        let sequence = (elapsed.as_nanos() / self.refresh.as_nanos()) as u64 + 1;
        self.vblank(sequence)
    }

    fn vblank(&self, sequence: u64) -> Vblank {
        let nanos = self.refresh.as_nanos() * u128::from(sequence);
        Vblank {
            sequence,
            time: self.origin + Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX)),
        }
    }

    pub fn predicted_render(&self) -> Duration {
        self.render_times.iter().copied().max().unwrap_or_default()
    }

    pub fn deadline(&self, vblank: Vblank) -> Duration {
        vblank
            .time
            .saturating_sub(self.predicted_render() + SAFETY_MARGIN)
    }

    pub fn frame_for(&self, now: Duration) -> Vblank {
        let first = self.vblank_after(now);
        if now <= self.deadline(first) {
            first
        } else {
            self.vblank(first.sequence + 1)
        }
    }

    pub fn record_render(&mut self, deadline_to_done: Duration) {
        if self.render_times.len() == REMEMBERED_RENDERS {
            self.render_times.pop_front();
        }
        self.render_times.push_back(deadline_to_done);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Waiting(Vblank),
    Rendered(Vblank),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Poll {
    pub wake_at: Option<Duration>,
    pub presented: Option<Vblank>,
}

pub struct FrameScheduler {
    clock: FrameClock,
    phase: Phase,
    wanted: bool,
}

impl FrameScheduler {
    pub fn new(clock: FrameClock) -> Self {
        Self {
            clock,
            phase: Phase::Idle,
            wanted: false,
        }
    }

    pub fn clock(&self) -> &FrameClock {
        &self.clock
    }

    pub fn poll(
        &mut self,
        clock: impl Fn() -> Duration,
        compositor: &mut Headless,
        renderer: &mut Renderer,
    ) -> Result<Poll, RenderError> {
        let mut now = clock();
        self.wanted |= compositor.take_redraw();
        let mut presented = None;
        loop {
            match self.phase {
                Phase::Idle if !self.wanted => {
                    return Ok(Poll {
                        wake_at: None,
                        presented,
                    });
                }
                Phase::Idle => self.phase = Phase::Waiting(self.clock.frame_for(now)),
                Phase::Waiting(vblank) if now >= vblank.time => {
                    self.phase = Phase::Waiting(self.clock.frame_for(now));
                }
                Phase::Waiting(vblank) if now < self.clock.deadline(vblank) => {
                    return Ok(Poll {
                        wake_at: Some(self.clock.deadline(vblank)),
                        presented,
                    });
                }
                Phase::Waiting(vblank) => {
                    compositor.take_redraw();
                    self.wanted = false;
                    compositor.latch();
                    let deadline = self.clock.deadline(vblank);
                    renderer.render(compositor)?;
                    now = clock();
                    self.clock.record_render(now.saturating_sub(deadline));
                    self.phase = if now <= vblank.time {
                        Phase::Rendered(vblank)
                    } else {
                        Phase::Rendered(self.clock.vblank_after(now))
                    };
                }
                Phase::Rendered(vblank) if now < vblank.time => {
                    return Ok(Poll {
                        wake_at: Some(vblank.time),
                        presented,
                    });
                }
                Phase::Rendered(vblank) => {
                    compositor.present(vblank).map_err(RenderError::Io)?;
                    presented = Some(vblank);
                    self.phase = Phase::Idle;
                }
            }
        }
    }
}
