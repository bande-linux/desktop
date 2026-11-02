use std::collections::VecDeque;
use std::io;
use std::time::Duration;

use crate::state::Compositor;

pub const SAFETY_MARGIN: Duration = Duration::from_millis(1);

const REMEMBERED_RENDERS: usize = 16;
const TYPICAL_RENDER_WEIGHT: f64 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vblank {
    pub sequence: u64,
    pub time: Duration,
}

pub struct FrameClock {
    origin: Duration,
    refresh: Duration,
    deadline_to_done: VecDeque<Duration>,
    typical_render: Option<Duration>,
}

impl FrameClock {
    pub fn new(origin: Duration, refresh: Duration) -> Self {
        Self {
            origin,
            refresh,
            deadline_to_done: VecDeque::with_capacity(REMEMBERED_RENDERS),
            typical_render: None,
        }
    }

    pub fn refresh(&self) -> Duration {
        self.refresh
    }

    pub fn anchor(&mut self, vblank: Duration) {
        self.origin = vblank;
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

    fn within_a_refresh(&self, time: Duration) -> Duration {
        time.min(self.refresh.saturating_sub(SAFETY_MARGIN))
    }

    fn predicted_render(&self) -> Duration {
        let slowest = self.deadline_to_done.iter().copied().max();
        self.within_a_refresh(slowest.unwrap_or_default())
    }

    pub fn deadline(&self, vblank: Vblank) -> Duration {
        vblank
            .time
            .saturating_sub(self.predicted_render() + SAFETY_MARGIN)
    }

    fn latest_start(&self, vblank: Vblank) -> Duration {
        vblank.time.saturating_sub(
            self.typical_render
                .unwrap_or_default()
                .min(self.predicted_render()),
        )
    }

    pub fn frame_for(&self, now: Duration) -> Vblank {
        let first = self.vblank_after(now);
        if now <= self.deadline(first) {
            first
        } else {
            self.vblank(first.sequence + 1)
        }
    }

    pub fn record_render(&mut self, deadline: Duration, started: Duration, done: Duration) {
        if self.deadline_to_done.len() == REMEMBERED_RENDERS {
            self.deadline_to_done.pop_front();
        }
        self.deadline_to_done
            .push_back(done.saturating_sub(deadline));
        let render = done.saturating_sub(started);
        self.typical_render = Some(self.typical_render.map_or(render, |typical| {
            let (typical, render) = (typical.as_secs_f64(), render.as_secs_f64());
            Duration::from_secs_f64(typical + (render - typical) * TYPICAL_RENDER_WEIGHT)
        }));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submission {
    AtVblank,
    AwaitingFlip,
    Paused,
}

pub trait FrameOutput {
    type Error: From<io::Error>;

    fn submit(
        &mut self,
        compositor: &mut Compositor,
        output: &str,
    ) -> Result<Submission, Self::Error>;

    fn set_color_temperature(&mut self, _kelvin: f64) {}

    fn switch_virtual_terminal(&mut self, _number: i32) {}

    fn renew_lost_renderer(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Waiting(Vblank),
    Rendered(Vblank),
    Held,
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
    output: Option<String>,
}

impl FrameScheduler {
    pub fn new(clock: FrameClock) -> Self {
        Self {
            clock,
            phase: Phase::Idle,
            wanted: false,
            output: None,
        }
    }

    pub fn for_output(clock: FrameClock, output: &str) -> Self {
        Self {
            output: Some(output.to_owned()),
            ..Self::new(clock)
        }
    }

    pub fn output(&self) -> Option<&str> {
        self.output.as_deref()
    }

    pub fn clock(&self) -> &FrameClock {
        &self.clock
    }

    pub fn poll<O: FrameOutput>(
        &mut self,
        clock: impl Fn() -> Duration,
        compositor: &mut Compositor,
        output: &mut O,
    ) -> Result<Poll, O::Error> {
        let mut now = clock();
        let name = self
            .output
            .clone()
            .unwrap_or_else(|| compositor.main_output());
        self.wanted |= compositor.take_redraw_on(&name);
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
                Phase::Waiting(vblank) if now > self.clock.latest_start(vblank) => {
                    self.phase = Phase::Waiting(self.clock.frame_for(now));
                }
                Phase::Waiting(vblank) if now < self.clock.deadline(vblank) => {
                    return Ok(Poll {
                        wake_at: Some(self.clock.deadline(vblank)),
                        presented,
                    });
                }
                Phase::Waiting(vblank) => {
                    let started = now;
                    compositor.take_redraw_on(&name);
                    self.wanted = false;
                    compositor.animate_on(&name, vblank.time);
                    compositor.latch_on(&name);
                    let deadline = self.clock.deadline(vblank);
                    let submission = output.submit(compositor, &name).inspect_err(|_| {
                        self.phase = Phase::Idle;
                    })?;
                    now = clock();
                    self.clock.record_render(deadline, started, now);
                    self.phase = match submission {
                        Submission::AwaitingFlip | Submission::Paused => Phase::Held,
                        Submission::AtVblank if now <= vblank.time => Phase::Rendered(vblank),
                        Submission::AtVblank => Phase::Rendered(self.clock.vblank_after(now)),
                    };
                }
                Phase::Held => {
                    return Ok(Poll {
                        wake_at: None,
                        presented,
                    });
                }
                Phase::Rendered(vblank) if now < vblank.time => {
                    return Ok(Poll {
                        wake_at: Some(vblank.time),
                        presented,
                    });
                }
                Phase::Rendered(vblank) => {
                    compositor.present_on(&name, vblank)?;
                    presented = Some(vblank);
                    self.wanted |= compositor.take_redraw_on(&name);
                    self.phase = Phase::Idle;
                }
            }
        }
    }

    pub fn resume(&mut self) {
        self.phase = Phase::Idle;
        self.wanted = true;
    }

    pub fn flipped(&mut self, vblank: Vblank, compositor: &mut Compositor) -> io::Result<()> {
        let name = self
            .output
            .clone()
            .unwrap_or_else(|| compositor.main_output());
        compositor.present_on(&name, vblank)?;
        self.clock.anchor(vblank.time);
        self.phase = Phase::Idle;
        Ok(())
    }
}
