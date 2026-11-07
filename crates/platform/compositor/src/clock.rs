use std::collections::VecDeque;
use std::io;
use std::time::Duration;

use crate::pacing::{Commit, FramePacing};
use crate::state::Compositor;

const SAFETY_MARGIN: Duration = Duration::from_millis(1);
const FLIP_TIMEOUT: Duration = Duration::from_secs(1);
const FAILED_FRAME_RETRY: Duration = Duration::from_millis(250);
const RESTART_DISPLAY_AFTER: Duration = Duration::from_secs(3);
const GIVE_UP_AFTER: Duration = Duration::from_secs(15);

const REMEMBERED_RENDERS: usize = 16;
const TYPICAL_RENDER_WEIGHT: f64 = 0.1;
const TIMING_VARIABLE: &str = "BANDE_FRAME_TIMING";
const CLASSIC_TIMING: &str = "classic";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vblank {
    pub sequence: u64,
    pub time: Duration,
}

pub struct FrameClock {
    timing: Timing,
}

enum Timing {
    Measured(Box<FramePacing>),
    Classic(ClassicClock),
}

struct ClassicClock {
    origin: Duration,
    refresh: Duration,
    deadline_to_done: VecDeque<Duration>,
    typical_render: Option<Duration>,
}

impl ClassicClock {
    fn vblank_after(&self, now: Duration) -> Vblank {
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

    fn deadline(&self, vblank: Vblank) -> Duration {
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

    fn frame_for(&self, now: Duration) -> Vblank {
        let first = self.vblank_after(now);
        if now <= self.deadline(first) {
            first
        } else {
            self.vblank(first.sequence + 1)
        }
    }

    fn record_render(&mut self, deadline: Duration, started: Duration, done: Duration) {
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

impl FrameClock {
    pub fn new(origin: Duration, refresh: Duration) -> Self {
        if std::env::var_os(TIMING_VARIABLE).is_some_and(|value| value == CLASSIC_TIMING) {
            Self::classic(origin, refresh)
        } else {
            Self::measured(origin, refresh)
        }
    }

    fn measured(origin: Duration, refresh: Duration) -> Self {
        Self {
            timing: Timing::Measured(Box::new(FramePacing::new(origin, refresh))),
        }
    }

    fn classic(origin: Duration, refresh: Duration) -> Self {
        Self {
            timing: Timing::Classic(ClassicClock {
                origin,
                refresh,
                deadline_to_done: VecDeque::with_capacity(REMEMBERED_RENDERS),
                typical_render: None,
            }),
        }
    }

    fn retimed(&self, origin: Duration, refresh: Duration) -> Self {
        match &self.timing {
            Timing::Measured(pacing) => Self {
                timing: Timing::Measured(Box::new(pacing.retimed(origin, refresh))),
            },
            Timing::Classic(_) => Self::classic(origin, refresh),
        }
    }

    pub fn refresh(&self) -> Duration {
        match &self.timing {
            Timing::Measured(pacing) => pacing.nominal(),
            Timing::Classic(classic) => classic.refresh,
        }
    }

    pub fn vblank_after(&self, now: Duration) -> Vblank {
        match &self.timing {
            Timing::Measured(pacing) => {
                let (sequence, time) = pacing.slot_after(now);
                Vblank { sequence, time }
            }
            Timing::Classic(classic) => classic.vblank_after(now),
        }
    }

    pub fn deadline(&self, vblank: Vblank) -> Duration {
        match &self.timing {
            Timing::Measured(pacing) => pacing.start_for(vblank.time),
            Timing::Classic(classic) => classic.deadline(vblank),
        }
    }

    fn latest_start(&self, vblank: Vblank) -> Duration {
        match &self.timing {
            Timing::Measured(pacing) => pacing.latest_start_for(vblank.time),
            Timing::Classic(classic) => classic.latest_start(vblank),
        }
    }

    pub fn frame_for(&self, now: Duration) -> Vblank {
        match &self.timing {
            Timing::Measured(pacing) => {
                let (sequence, time) = pacing.slot_after(now);
                if now <= pacing.latest_start_for(time) {
                    Vblank { sequence, time }
                } else {
                    let sequence = sequence + 1;
                    Vblank {
                        sequence,
                        time: pacing.slot(sequence),
                    }
                }
            }
            Timing::Classic(classic) => classic.frame_for(now),
        }
    }

    fn woke_late(&mut self, by: Duration) {
        if let Timing::Measured(pacing) = &mut self.timing {
            pacing.prediction.record_wake(by);
        }
    }

    fn record_render(&mut self, deadline: Duration, started: Duration, done: Duration, drew: bool) {
        match &mut self.timing {
            Timing::Measured(pacing) => {
                if drew {
                    pacing
                        .prediction
                        .record_render(done.saturating_sub(started));
                }
            }
            Timing::Classic(classic) => classic.record_render(deadline, started, done),
        }
    }

    fn flipped(&mut self, vblank: Vblank, commit: Option<Commit>) -> Vblank {
        match &mut self.timing {
            Timing::Measured(pacing) => {
                let (sequence, time) = pacing.flipped(vblank.time, commit);
                Vblank { sequence, time }
            }
            Timing::Classic(classic) => {
                classic.origin = vblank.time;
                vblank
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submission {
    AtVblank,
    Unchanged,
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

    fn can_dim(&mut self, _output: &str) -> bool {
        false
    }

    fn set_dim(&mut self, _output: &str, _gain: f64) {}

    fn switch_virtual_terminal(&mut self, _number: i32) {}

    fn renew_failed_renderer(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }

    fn refresh(&self, _output: &str) -> Option<Duration> {
        None
    }

    fn flip_missed(&mut self, _output: &str) {}

    fn restart_display(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryStep {
    Wait,
    RestartDisplay,
    GiveUp,
}

#[derive(Debug, Default)]
pub struct FrameRecovery {
    restarted: bool,
}

impl FrameRecovery {
    pub fn step(&mut self, stalled_for: Option<Duration>) -> RecoveryStep {
        let Some(stalled_for) = stalled_for else {
            self.restarted = false;
            return RecoveryStep::Wait;
        };
        if stalled_for >= GIVE_UP_AFTER {
            RecoveryStep::GiveUp
        } else if stalled_for >= RESTART_DISPLAY_AFTER && !self.restarted {
            self.restarted = true;
            RecoveryStep::RestartDisplay
        } else {
            RecoveryStep::Wait
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Waiting(Vblank),
    Rendered(Vblank),
    Held,
    AwaitingFlip(Duration),
    Backoff(Duration),
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
    stalled_since: Option<Duration>,
    asleep_until: Option<Duration>,
    committed: Option<Commit>,
}

impl FrameScheduler {
    pub fn new(clock: FrameClock) -> Self {
        Self {
            clock,
            phase: Phase::Idle,
            wanted: false,
            output: None,
            stalled_since: None,
            asleep_until: None,
            committed: None,
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

    pub fn stalled_since(&self) -> Option<Duration> {
        self.stalled_since
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
                    let wake_at = self.clock.deadline(vblank);
                    self.asleep_until = Some(wake_at);
                    return Ok(Poll {
                        wake_at: Some(wake_at),
                        presented,
                    });
                }
                Phase::Waiting(vblank) => {
                    let started = now;
                    if let Some(planned) = self.asleep_until.take() {
                        self.clock.woke_late(started.saturating_sub(planned));
                    }
                    compositor.take_redraw_on(&name);
                    self.wanted = false;
                    compositor.animate_on(&name, vblank.time);
                    compositor.latch_on(&name);
                    let deadline = self.clock.deadline(vblank);
                    let submission = output.submit(compositor, &name).inspect_err(|_| {
                        self.wanted = true;
                        self.stalled_since.get_or_insert(started);
                        self.phase = Phase::Backoff(started + FAILED_FRAME_RETRY);
                    })?;
                    now = clock();
                    self.clock.record_render(
                        deadline,
                        started,
                        now,
                        submission != Submission::Unchanged,
                    );
                    self.committed = (submission == Submission::AwaitingFlip).then_some(Commit {
                        sequence: vblank.sequence,
                        at: now,
                    });
                    if let Some(refresh) = output.refresh(&name)
                        && refresh != self.clock.refresh()
                    {
                        self.clock = self.clock.retimed(now, refresh);
                    }
                    let next_vblank = if now <= vblank.time {
                        vblank
                    } else {
                        self.clock.vblank_after(now)
                    };
                    self.phase = match submission {
                        Submission::AwaitingFlip => Phase::AwaitingFlip(now),
                        Submission::Paused => {
                            self.stalled_since = None;
                            Phase::Held
                        }
                        Submission::AtVblank => {
                            self.stalled_since = None;
                            Phase::Rendered(next_vblank)
                        }
                        Submission::Unchanged => Phase::Rendered(next_vblank),
                    };
                }
                Phase::Held => {
                    return Ok(Poll {
                        wake_at: None,
                        presented,
                    });
                }
                Phase::AwaitingFlip(since) if now < since + FLIP_TIMEOUT => {
                    return Ok(Poll {
                        wake_at: Some(since + FLIP_TIMEOUT),
                        presented,
                    });
                }
                Phase::AwaitingFlip(since) => {
                    tracing::warn!("page flip on {name} timed out");
                    self.stalled_since.get_or_insert(since);
                    output.flip_missed(&name);
                    self.phase = Phase::Idle;
                    self.wanted = true;
                }
                Phase::Backoff(until) if now < until => {
                    return Ok(Poll {
                        wake_at: Some(until),
                        presented,
                    });
                }
                Phase::Backoff(_) => self.phase = Phase::Idle,
                Phase::Rendered(vblank) if now < vblank.time => {
                    return Ok(Poll {
                        wake_at: Some(vblank.time),
                        presented,
                    });
                }
                Phase::Rendered(vblank) => {
                    compositor.present_on(&name, vblank, false)?;
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
        self.stalled_since = None;
    }

    pub fn restart(&mut self) {
        self.phase = Phase::Idle;
        self.wanted = true;
    }

    pub fn flipped(&mut self, vblank: Vblank, compositor: &mut Compositor) -> io::Result<()> {
        let name = self
            .output
            .clone()
            .unwrap_or_else(|| compositor.main_output());
        let vblank = self.clock.flipped(vblank, self.committed.take());
        compositor.present_on(&name, vblank, true)?;
        self.phase = Phase::Idle;
        self.stalled_since = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUTOFF: Duration = Duration::from_micros(2300);
    const REPORTED_EARLY: Duration = Duration::from_millis(1);
    const FRAMES: u32 = 4000;
    const LATE_CALLBACKS: f64 = 0.01;
    const TRUE_RATE_OFF: f64 = 1.0002;

    #[derive(Debug, Default)]
    struct Run {
        frames: u32,
        missed: u32,
        one_refresh: u32,
        left_on_hit: Duration,
    }

    struct Noise(u32);

    impl Noise {
        fn next(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            f64::from(self.0 >> 8) / f64::from(1u32 << 24)
        }
    }

    fn render(noise: &mut Noise, typical: Duration) -> Duration {
        let wobble = 1.0 + noise.next() * 0.3;
        let spike = if noise.next() < 0.01 { 2.5 } else { 1.0 };
        typical.mul_f64(wobble * spike)
    }

    fn display(mut clock: FrameClock, nominal: Duration, typical: Duration) -> Run {
        let period = nominal.mul_f64(TRUE_RATE_OFF);
        let mut noise = Noise(7);
        let origin = Duration::from_secs(1);
        let mut run = Run::default();
        let mut now = origin;
        let mut last_flip_slot = None;
        let mut left_total = Duration::ZERO;
        let mut hits = 0u32;
        for _ in 0..FRAMES {
            let vblank = clock.frame_for(now);
            let mut frame = vblank;
            let deadline = clock.deadline(frame);
            if now < deadline {
                now = deadline + Duration::from_micros(50);
                clock.woke_late(now - deadline);
            }
            if now > clock.latest_start(frame) {
                frame = clock.frame_for(now);
            }
            let started = now;
            let done = started + render(&mut noise, typical);
            clock.record_render(clock.deadline(frame), started, done, true);
            let next_slot = done
                .saturating_sub(origin)
                .as_nanos()
                .div_ceil(period.as_nanos());
            let aimed = origin + period * next_slot as u32;
            let (slot, flip) = if aimed.saturating_sub(done) >= CUTOFF {
                (next_slot, aimed)
            } else {
                (next_slot + 1, aimed + period)
            };
            let wanted = frame.time + REPORTED_EARLY;
            run.frames += 1;
            if flip > wanted + period / 2 {
                run.missed += 1;
            } else {
                left_total += flip.saturating_sub(done);
                hits += 1;
            }
            if last_flip_slot.is_some_and(|last| slot == last + 1) {
                run.one_refresh += 1;
            }
            last_flip_slot = Some(slot);
            let late_callback = if noise.next() < LATE_CALLBACKS {
                Duration::from_millis(1)
            } else {
                Duration::ZERO
            };
            clock.flipped(
                Vblank {
                    sequence: 0,
                    time: flip - REPORTED_EARLY + late_callback,
                },
                Some(Commit {
                    sequence: frame.sequence,
                    at: done,
                }),
            );
            now = flip;
        }
        run.left_on_hit = left_total / hits.max(1);
        run
    }

    fn share(part: u32, whole: u32) -> f64 {
        f64::from(part) / f64::from(whole)
    }

    #[test]
    fn a_display_with_a_commit_cutoff_keeps_every_refresh_at_any_rate() {
        for hertz in [60.0, 75.0, 119.989, 144.001, 240.0] {
            let period = Duration::from_secs_f64(1.0 / hertz);
            let typical = Duration::from_micros(800);
            let old = display(
                FrameClock::classic(Duration::from_secs(1), period),
                period,
                typical,
            );
            let new = display(
                FrameClock::measured(Duration::from_secs(1), period),
                period,
                typical,
            );
            println!("{hertz} Hz old {old:?}");
            println!("{hertz} Hz new {new:?}");
            assert!(share(new.missed, new.frames) < 0.02, "{hertz} Hz: {new:?}");
            assert!(
                share(new.one_refresh, new.frames) > 0.97,
                "{hertz} Hz: {new:?}"
            );
            assert!(share(old.one_refresh, old.frames) < share(new.one_refresh, new.frames));
        }
    }

    #[test]
    fn heavy_frames_at_144_hz_use_the_time_left_after_the_cutoff() {
        let period = Duration::from_secs_f64(1.0 / 144.001);
        let run = display(
            FrameClock::measured(Duration::from_secs(1), period),
            period,
            Duration::from_micros(2800),
        );
        assert!(share(run.one_refresh, run.frames) > 0.9, "{run:?}");
    }

    #[test]
    fn a_mode_change_keeps_what_was_learned_about_rendering() {
        let at_120 = Duration::from_secs_f64(1.0 / 119.989);
        let at_60 = Duration::from_secs_f64(1.0 / 60.0);
        let mut clock = FrameClock::measured(Duration::from_secs(1), at_120);
        clock.record_render(
            Duration::ZERO,
            Duration::ZERO,
            Duration::from_millis(3),
            true,
        );
        let budget = clock.deadline(Vblank {
            sequence: 1,
            time: Duration::from_secs(5),
        });
        let changed = clock.retimed(Duration::from_secs(2), at_60);
        assert_eq!(changed.refresh(), at_60);
        assert_eq!(
            changed.deadline(Vblank {
                sequence: 1,
                time: Duration::from_secs(5),
            }),
            budget
        );
    }
}
