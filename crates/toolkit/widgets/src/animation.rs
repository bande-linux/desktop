use std::time::Instant;

use unbound_spring::{Spring, State};

use super::Context;

const SETTLED_DISTANCE: f64 = 0.001;
const SETTLED_SPEED: f64 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Animated {
    spring: Spring,
    start: State,
    target: f64,
    since: Option<Instant>,
}

impl Default for Animated {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl Animated {
    pub const fn new(value: f32) -> Self {
        Self {
            spring: Spring::QUICK,
            start: State {
                position: value as f64,
                velocity: 0.0,
            },
            target: value as f64,
            since: None,
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
        if self.animate_at(cx.now(), cx.reduce_motion, target, spring) {
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
        *self = Self::new(value);
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
        let settled = (state.position - self.target).abs() <= SETTLED_DISTANCE
            && state.velocity.abs() <= SETTLED_SPEED;
        if settled {
            (self.target as f32, false)
        } else {
            (state.position as f32, true)
        }
    }
}
