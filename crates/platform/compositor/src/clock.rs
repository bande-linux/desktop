use std::collections::VecDeque;
use std::io;
use std::time::Duration;

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
pub enum Submission {
    AtVblank,
    AwaitingFlip,
}

pub trait FrameOutput {
    type Error: From<io::Error>;

    fn submit(
        &mut self,
        compositor: &mut Headless,
        output: &str,
    ) -> Result<Submission, Self::Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Waiting(Vblank),
    Rendered(Vblank),
    Flipping,
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
        compositor: &mut Headless,
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
                    compositor.take_redraw_on(&name);
                    self.wanted = false;
                    compositor.animate(vblank.time);
                    compositor.latch_on(&name);
                    let deadline = self.clock.deadline(vblank);
                    let submission = output.submit(compositor, &name)?;
                    now = clock();
                    self.clock.record_render(now.saturating_sub(deadline));
                    self.phase = match submission {
                        Submission::AwaitingFlip => Phase::Flipping,
                        Submission::AtVblank if now <= vblank.time => Phase::Rendered(vblank),
                        Submission::AtVblank => Phase::Rendered(self.clock.vblank_after(now)),
                    };
                }
                Phase::Flipping => {
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

    pub fn flipped(&mut self, vblank: Vblank, compositor: &mut Headless) -> io::Result<()> {
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
