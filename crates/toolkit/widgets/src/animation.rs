use std::time::Instant;

use unbound_spring::{Spring, State};

use super::Context;

const UNIT_PRECISION: f64 = 1.0 / 512.0;
const POINT_PRECISION: f64 = 0.05;
const REST_HORIZON: f64 = 0.02;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Animated {
    spring: Spring,
    start: State,
    target: f64,
    since: Option<Instant>,
    precision: f64,
}

impl Default for Animated {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl Animated {
    pub const fn new(value: f32) -> Self {
        Self::with_precision(value, UNIT_PRECISION)
    }

    pub const fn points(value: f32) -> Self {
        Self::with_precision(value, POINT_PRECISION)
    }

    const fn with_precision(value: f32, precision: f64) -> Self {
        Self {
            spring: Spring::QUICK,
            start: State {
                position: value as f64,
                velocity: 0.0,
            },
            target: value as f64,
            since: None,
            precision,
        }
    }

    pub fn target(&self) -> f32 {
        self.target as f32
    }

    fn state(&self, now: Instant) -> State {
        match self.since {
            Some(since) => self.spring.state_at(
                self.start,
                self.target,
                now.saturating_duration_since(since).as_secs_f64(),
            ),
            None => self.start,
        }
    }

    pub fn animate_to(&mut self, cx: &Context, target: f32, spring: Spring) {
        if self.animate_at(cx.animation_start(), cx.reduce_motion, target, spring) {
            cx.animating.set(true);
        }
    }

    pub fn animate_at(
        &mut self,
        now: Instant,
        reduce_motion: bool,
        target: f32,
        spring: Spring,
    ) -> bool {
        let target = f64::from(target);
        if target == self.target {
            return false;
        }
        if reduce_motion && spring != Spring::FADE {
            self.jump_to(target as f32);
            return false;
        }
        self.start = self.state(now);
        self.spring = spring;
        self.target = target;
        self.since = Some(now);
        true
    }

    pub fn jump_to(&mut self, value: f32) {
        *self = Self::with_precision(value, self.precision);
    }

    pub fn value(&self, cx: &Context) -> f32 {
        let (value, moving) = self.value_at(cx.now());
        if moving {
            cx.animating.set(true);
        }
        value
    }

    pub fn value_at(&self, now: Instant) -> (f32, bool) {
        let state = self.state(now);
        let settled = (state.position - self.target).abs() <= self.precision
            && state.velocity.abs() * REST_HORIZON <= self.precision;
        if settled {
            (self.target as f32, false)
        } else {
            (state.position as f32, true)
        }
    }
}
