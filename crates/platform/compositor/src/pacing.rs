use std::time::{Duration, Instant};

const PHASE_GAIN: f64 = 1.0 / 16.0;
const PERIOD_GAIN: f64 = 1.0 / 256.0;
const PERIOD_RANGE: f64 = 0.01;
const OUTLIER_SHARE: f64 = 1.0 / 8.0;
const OUTLIERS_BEFORE_SNAP: u32 = 8;

const HISTORY: usize = 64;
const RECENT: usize = 4;
const PERCENTILE: usize = 95;
const WAKE_PERCENTILE: usize = 99;
const TYPICAL_PERCENTILE: usize = 50;

const FIRST_LEAD_SHARE: f64 = 0.35;
const LEAD_CEILING_SHARE: f64 = 0.5;
const LEARNING_STEP: Duration = Duration::from_micros(50);
const MISS_STEP: Duration = Duration::from_micros(100);
const MISS_STEP_LIMIT: Duration = Duration::from_micros(300);
const PROBE_STEP: Duration = Duration::from_micros(20);
const PROBE_AFTER_HITS: u32 = 1024;

const PAD_STEP: Duration = Duration::from_micros(100);
const PAD_CEILING_SHARE: f64 = 0.25;
const PAD_DECAY: Duration = Duration::from_micros(10);
const PAD_DECAY_AFTER_HITS: u32 = 64;
const RELIEF_SHARE: f64 = 0.125;
const WINDOW_FRAMES: u32 = 128;
const WINDOW_MISSES: u32 = 6;
const REANCHOR_AFTER_SLOTS: f64 = 64.0;

const CLIENT_GUARD: Duration = Duration::from_micros(500);
const CLIENT_SAMPLES_BEFORE_DELAY: usize = 8;
const CLIENT_BACK_OFF: Duration = Duration::from_secs(1);

fn share(period: Duration, of: f64) -> Duration {
    period.mul_f64(of)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct VsyncEstimate {
    nominal: Duration,
    period: f64,
    anchor: f64,
    sequence: u64,
    outliers: u32,
    observed: bool,
}

impl VsyncEstimate {
    pub fn new(origin: Duration, period: Duration) -> Self {
        Self {
            nominal: period,
            period: period.as_secs_f64(),
            anchor: origin.as_secs_f64(),
            sequence: 0,
            outliers: 0,
            observed: false,
        }
    }

    pub fn nominal(&self) -> Duration {
        self.nominal
    }

    pub fn observe(&mut self, flip: Duration) -> (u64, Duration) {
        let flip = flip.as_secs_f64();
        if !self.observed {
            self.observed = true;
            self.anchor = flip;
            self.sequence += 1;
            return (self.sequence, self.time_at(0));
        }
        let slots = ((flip - self.anchor) / self.period).round().max(0.0);
        let error = flip - (self.anchor + slots * self.period);
        if slots > REANCHOR_AFTER_SLOTS {
            self.outliers = 0;
            self.anchor = flip;
            self.sequence += slots as u64;
            return (self.sequence, self.time_at(0));
        }
        if error.abs() > self.period * OUTLIER_SHARE {
            self.outliers += 1;
            if self.outliers >= OUTLIERS_BEFORE_SNAP {
                self.outliers = 0;
                self.anchor = flip;
            } else {
                self.anchor += slots * self.period;
            }
            self.sequence += slots as u64;
            return (self.sequence, self.time_at(0));
        }
        self.outliers = 0;
        if slots >= 1.0 {
            let nominal = self.nominal.as_secs_f64();
            self.period = (self.period + error * PERIOD_GAIN / slots).clamp(
                nominal * (1.0 - PERIOD_RANGE),
                nominal * (1.0 + PERIOD_RANGE),
            );
        }
        self.anchor += slots * self.period + error * PHASE_GAIN;
        self.sequence += slots as u64;
        (self.sequence, self.time_at(0))
    }

    fn time_at(&self, slots: u64) -> Duration {
        Duration::from_secs_f64((self.anchor + slots as f64 * self.period).max(0.0))
    }

    pub fn slot_after(&self, now: Duration) -> (u64, Duration) {
        let elapsed = (now.as_secs_f64() - self.anchor).max(0.0);
        let slots = (elapsed / self.period).floor() as u64 + 1;
        (self.sequence + slots, self.time_at(slots))
    }

    pub fn slot(&self, sequence: u64) -> Duration {
        self.time_at(sequence.saturating_sub(self.sequence))
    }
}

#[derive(Debug, Clone)]
struct Samples {
    values: [Duration; HISTORY],
    next: usize,
    filled: usize,
}

impl Default for Samples {
    fn default() -> Self {
        Self {
            values: [Duration::ZERO; HISTORY],
            next: 0,
            filled: 0,
        }
    }
}

impl Samples {
    fn push(&mut self, value: Duration) {
        self.values[self.next] = value;
        self.next = (self.next + 1) % HISTORY;
        self.filled = (self.filled + 1).min(HISTORY);
    }

    fn percentile(&self, percent: usize) -> Duration {
        if self.filled == 0 {
            return Duration::ZERO;
        }
        let mut sorted = self.values;
        let filled = &mut sorted[..self.filled];
        filled.sort_unstable();
        filled[(self.filled - 1) * percent / 100]
    }

    fn recent_max(&self, count: usize) -> Duration {
        (0..count.min(self.filled))
            .map(|back| self.values[(self.next + HISTORY - 1 - back) % HISTORY])
            .max()
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default)]
pub struct RenderPrediction {
    renders: Samples,
    wake_delays: Samples,
    pad: Duration,
    hits: u32,
}

impl RenderPrediction {
    pub fn record_render(&mut self, render: Duration) {
        self.renders.push(render);
    }

    pub fn record_wake(&mut self, delay: Duration) {
        self.wake_delays.push(delay);
    }

    fn budget(&self) -> Duration {
        self.renders
            .percentile(PERCENTILE)
            .max(self.renders.recent_max(RECENT))
            + self.wake_delays.percentile(WAKE_PERCENTILE)
            + self.pad
    }

    fn typical(&self) -> Duration {
        self.renders.percentile(TYPICAL_PERCENTILE)
    }

    fn committed_late(&mut self, refresh: Duration) {
        self.hits = 0;
        self.pad = (self.pad + PAD_STEP).min(share(refresh, PAD_CEILING_SHARE));
    }

    fn committed_in_time(&mut self) {
        self.hits += 1;
        if self.hits >= PAD_DECAY_AFTER_HITS {
            self.hits = 0;
            self.pad = self.pad.saturating_sub(PAD_DECAY);
        }
    }

    fn relieve(&mut self, refresh: Duration) {
        self.pad = (self.pad + share(refresh, RELIEF_SHARE)).min(share(refresh, PAD_CEILING_SHARE));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommitLead {
    lead: Duration,
    learning: bool,
    hits: u32,
}

impl CommitLead {
    pub fn get(&self) -> Duration {
        self.lead
    }

    fn start(&mut self, refresh: Duration) {
        self.lead = share(refresh, FIRST_LEAD_SHARE);
        self.learning = true;
        self.hits = 0;
    }

    fn hit(&mut self) {
        if self.learning {
            self.lead = self.lead.saturating_sub(LEARNING_STEP);
            return;
        }
        self.hits += 1;
        if self.hits >= PROBE_AFTER_HITS {
            self.hits = 0;
            self.lead = self.lead.saturating_sub(PROBE_STEP);
        }
    }

    fn missed(&mut self, left: Duration, refresh: Duration) {
        self.learning = false;
        self.hits = 0;
        let wanted = left + MISS_STEP;
        self.lead = self
            .lead
            .max(wanted.min(self.lead + MISS_STEP_LIMIT))
            .min(share(refresh, LEAD_CEILING_SHARE));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Commit {
    pub sequence: u64,
    pub at: Duration,
}

#[derive(Debug, Clone)]
pub struct FramePacing {
    vsync: VsyncEstimate,
    pub prediction: RenderPrediction,
    lead: CommitLead,
    flips_seen: bool,
    window_frames: u32,
    window_misses: u32,
}

impl FramePacing {
    pub fn new(origin: Duration, refresh: Duration) -> Self {
        Self {
            vsync: VsyncEstimate::new(origin, refresh),
            prediction: RenderPrediction::default(),
            lead: CommitLead::default(),
            flips_seen: false,
            window_frames: 0,
            window_misses: 0,
        }
    }

    pub fn retimed(&self, origin: Duration, refresh: Duration) -> Self {
        Self {
            prediction: self.prediction.clone(),
            ..Self::new(origin, refresh)
        }
    }

    pub fn nominal(&self) -> Duration {
        self.vsync.nominal()
    }

    pub fn slot_after(&self, now: Duration) -> (u64, Duration) {
        self.vsync.slot_after(now)
    }

    pub fn slot(&self, sequence: u64) -> Duration {
        self.vsync.slot(sequence)
    }

    pub fn start_for(&self, slot: Duration) -> Duration {
        slot.saturating_sub(self.prediction.budget() + self.lead.get())
    }

    pub fn latest_start_for(&self, slot: Duration) -> Duration {
        slot.saturating_sub(self.prediction.typical() + self.lead.get())
    }

    pub fn flipped(&mut self, flip: Duration, commit: Option<Commit>) -> (u64, Duration) {
        let refresh = self.vsync.nominal();
        let target = commit.map(|commit| (commit, self.vsync.slot(commit.sequence)));
        let (sequence, time) = self.vsync.observe(flip);
        if !self.flips_seen {
            self.flips_seen = true;
            self.lead.start(refresh);
            return (sequence, time);
        }
        let Some((commit, target)) = target else {
            return (sequence, time);
        };
        if sequence > commit.sequence {
            let left = target.saturating_sub(commit.at);
            if left >= self.lead.get() {
                self.lead.missed(left, refresh);
            } else {
                self.prediction.committed_late(refresh);
            }
            self.window_misses += 1;
        } else {
            self.lead.hit();
            self.prediction.committed_in_time();
        }
        self.window_frames += 1;
        if self.window_frames >= WINDOW_FRAMES {
            if self.window_misses > WINDOW_MISSES {
                tracing::debug!(
                    "{} of {WINDOW_FRAMES} frames missed their flip; starting earlier",
                    self.window_misses
                );
                self.prediction.relieve(refresh);
            }
            self.window_frames = 0;
            self.window_misses = 0;
        }
        (sequence, time)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ClientPace {
    sent_at: Option<Instant>,
    delayed: bool,
    costs: Samples,
    back_off_until: Option<Instant>,
}

impl ClientPace {
    pub(crate) fn committed(&mut self, now: Instant) {
        if let Some(sent) = self.sent_at.take() {
            self.costs.push(now.saturating_duration_since(sent));
        }
    }

    pub(crate) fn latched(&mut self, now: Instant) {
        if self.delayed && self.sent_at.is_some() {
            self.back_off_until = Some(now + CLIENT_BACK_OFF);
        }
    }

    pub(crate) fn sent(&mut self, now: Instant, delayed: bool) {
        self.sent_at = Some(now);
        self.delayed = delayed;
    }

    pub(crate) fn time_needed(&self, now: Instant) -> Option<Duration> {
        let backing_off = self.back_off_until.is_some_and(|until| now < until);
        (!backing_off && self.costs.filled >= CLIENT_SAMPLES_BEFORE_DELAY)
            .then(|| self.costs.percentile(PERCENTILE) + CLIENT_GUARD)
    }
}
