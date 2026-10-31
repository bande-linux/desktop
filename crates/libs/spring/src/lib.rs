use std::f64::consts::TAU;
use std::fmt;
use std::ops::Sub;
use std::time::Duration;

const CRITICAL_BAND: f64 = 1e-6;
const REST_HORIZON: f64 = 0.02;
const UNIT_PRECISION: f64 = 1.0 / 512.0;
const POINT_PRECISION: f64 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct State {
    pub position: f64,
    pub velocity: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    angular_frequency: f64,
    damping_ratio: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpringError {
    AngularFrequency(f64),
    DampingRatio(f64),
}

impl fmt::Display for SpringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AngularFrequency(value) => {
                write!(
                    f,
                    "angular frequency must be finite and positive, got {value}"
                )
            }
            Self::DampingRatio(value) => {
                write!(
                    f,
                    "damping ratio must be finite and non-negative, got {value}"
                )
            }
        }
    }
}

impl std::error::Error for SpringError {}

impl Spring {
    pub const INTERACTIVE: Self = Self::preset(0.15, 0.86);
    pub const QUICK: Self = Self::preset(0.18, 1.0);
    pub const STANDARD: Self = Self::preset(0.25, 1.0);
    pub const GENTLE: Self = Self::preset(0.35, 1.0);
    pub const MOMENTUM: Self = Self::preset(0.3, 0.85);
    pub const BOUNCY: Self = Self::preset(0.4, 0.7);
    pub const SHAKE: Self = Self::preset(0.12, 0.2);
    pub const FADE: Self = Self::preset(0.12, 1.0);

    const fn preset(duration: f64, damping_ratio: f64) -> Self {
        Self {
            angular_frequency: TAU / duration,
            damping_ratio,
        }
    }

    pub fn new(angular_frequency: f64, damping_ratio: f64) -> Result<Self, SpringError> {
        if !(angular_frequency.is_finite() && angular_frequency > 0.0) {
            return Err(SpringError::AngularFrequency(angular_frequency));
        }
        if !(damping_ratio.is_finite() && damping_ratio >= 0.0) {
            return Err(SpringError::DampingRatio(damping_ratio));
        }
        Ok(Self {
            angular_frequency,
            damping_ratio,
        })
    }

    pub fn angular_frequency(&self) -> f64 {
        self.angular_frequency
    }

    pub fn damping_ratio(&self) -> f64 {
        self.damping_ratio
    }

    pub fn state_at(&self, start: State, target: f64, time: f64) -> State {
        let omega = self.angular_frequency;
        let zeta = self.damping_ratio;
        let d0 = start.position - target;
        let v0 = start.velocity;
        let (displacement, velocity) = if (zeta - 1.0).abs() < CRITICAL_BAND {
            let decay = (-omega * time).exp();
            let slope = v0 + omega * d0;
            (
                (d0 + slope * time) * decay,
                (v0 - omega * slope * time) * decay,
            )
        } else if zeta < 1.0 {
            let damped = omega * (1.0 - zeta * zeta).sqrt();
            let decay = (-zeta * omega * time).exp();
            let (sin, cos) = (damped * time).sin_cos();
            (
                decay * (d0 * cos + (v0 + zeta * omega * d0) / damped * sin),
                decay * (v0 * cos - (omega * omega * d0 + zeta * omega * v0) / damped * sin),
            )
        } else {
            let root = (zeta * zeta - 1.0).sqrt();
            let fast = -omega * (zeta + root);
            let slow = -omega * (zeta - root);
            let slow_weight = (v0 - fast * d0) / (slow - fast);
            let fast_weight = d0 - slow_weight;
            let (slow_term, fast_term) = ((slow * time).exp(), (fast * time).exp());
            (
                slow_weight * slow_term + fast_weight * fast_term,
                slow_weight * slow * slow_term + fast_weight * fast * fast_term,
            )
        };
        State {
            position: target + displacement,
            velocity,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Animation {
    spring: Spring,
    start: State,
    target: f64,
    elapsed: f64,
}

impl Animation {
    pub fn new(spring: Spring, start: State, target: f64) -> Self {
        Self {
            spring,
            start,
            target,
            elapsed: 0.0,
        }
    }

    pub fn state(&self) -> State {
        self.spring.state_at(self.start, self.target, self.elapsed)
    }

    pub fn target(&self) -> f64 {
        self.target
    }

    pub fn advance(&mut self, seconds: f64) -> State {
        self.elapsed += seconds;
        self.state()
    }

    pub fn retarget(&mut self, target: f64) {
        self.start = self.state();
        self.target = target;
        self.elapsed = 0.0;
    }

    pub fn is_at_rest(&self, precision: f64) -> bool {
        is_settled(self.state(), self.target, precision)
    }
}

fn is_settled(state: State, target: f64, precision: f64) -> bool {
    (state.position - target).abs() <= precision && state.velocity.abs() * REST_HORIZON <= precision
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Animated<T> {
    spring: Spring,
    start: State,
    target: f64,
    since: Option<T>,
    precision: f64,
}

impl<T> Default for Animated<T> {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl<T> Animated<T> {
    pub fn new(value: impl Into<f64>) -> Self {
        Self::with_precision(value.into(), UNIT_PRECISION)
    }

    pub fn points(value: impl Into<f64>) -> Self {
        Self::with_precision(value.into(), POINT_PRECISION)
    }

    const fn with_precision(value: f64, precision: f64) -> Self {
        Self {
            spring: Spring::QUICK,
            start: State {
                position: value,
                velocity: 0.0,
            },
            target: value,
            since: None,
            precision,
        }
    }

    pub fn target(&self) -> f32 {
        self.target as f32
    }

    pub fn jump_to(&mut self, value: impl Into<f64>) {
        *self = Self::with_precision(value.into(), self.precision);
    }
}

impl<T: Copy + PartialOrd + Sub<Output = Duration>> Animated<T> {
    pub fn state_at(&self, now: T) -> State {
        match self.since {
            Some(since) if now > since => {
                self.spring
                    .state_at(self.start, self.target, (now - since).as_secs_f64())
            }
            _ => self.start,
        }
    }

    pub fn animate_at(
        &mut self,
        now: T,
        reduce_motion: bool,
        target: impl Into<f64>,
        spring: Spring,
    ) -> bool {
        let target = target.into();
        if target == self.target {
            return false;
        }
        if reduce_motion && spring != Spring::FADE {
            self.jump_to(target);
            return false;
        }
        self.start = self.state_at(now);
        self.spring = spring;
        self.target = target;
        self.since = Some(now);
        true
    }

    pub fn is_at_rest(&self, now: T, precision: f64) -> bool {
        is_settled(self.state_at(now), self.target, precision)
    }

    pub fn value_at(&self, now: T) -> (f32, bool) {
        let state = self.state_at(now);
        if is_settled(state, self.target, self.precision) {
            (self.target as f32, false)
        } else {
            (state.position as f32, true)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decay {
    rate_per_millisecond: f64,
}

impl Decay {
    pub const NORMAL: Self = Self {
        rate_per_millisecond: 0.998,
    };

    pub fn state_at(&self, start: State, seconds: f64) -> State {
        let per_second = self.rate_per_millisecond.ln() * 1000.0;
        let remaining = (per_second * seconds).exp();
        State {
            position: start.position + start.velocity * (remaining - 1.0) / per_second,
            velocity: start.velocity * remaining,
        }
    }

    pub fn project(&self, start: State) -> f64 {
        start.position - start.velocity / (self.rate_per_millisecond.ln() * 1000.0)
    }
}

pub const RUBBER_BAND_CONSTANT: f64 = 0.55;

pub fn rubber_band(overshoot: f64, dimension: f64) -> f64 {
    let stretched = RUBBER_BAND_CONSTANT * overshoot.abs();
    (dimension * stretched / (dimension + stretched)).copysign(overshoot)
}
