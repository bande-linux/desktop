use std::time::Duration;

use smithay::utils::{Logical, Physical, Point};
use unbound_config::ZOOM_RANGE;
use unbound_spring::{Animation, Spring, State as Phase};
use zbus::zvariant::Value;

use crate::state::{Headless, State};

const STEP: f64 = 1.25;
const FIRST_LEVEL: f64 = 2.0;
const SETTLED: f64 = 0.0005;

pub(crate) struct Zoom {
    level: Animation,
    last: Option<Duration>,
    remembered: f64,
    written: Vec<f64>,
    focus: Option<Point<f64, Logical>>,
}

impl Default for Zoom {
    fn default() -> Self {
        Self {
            level: still(1.0),
            last: None,
            remembered: FIRST_LEVEL,
            written: Vec::new(),
            focus: None,
        }
    }
}

fn still(level: f64) -> Animation {
    Animation::new(
        Spring::QUICK,
        Phase {
            position: level,
            velocity: 0.0,
        },
        level,
    )
}

impl Zoom {
    pub(crate) fn level(&self) -> f64 {
        self.level.state().position.max(1.0)
    }

    pub(crate) fn target(&self) -> f64 {
        self.level.target()
    }

    pub(crate) fn is_animating(&self) -> bool {
        !self.level.is_settled(SETTLED, SETTLED)
    }

    fn go_to(&mut self, level: f64, reduce_motion: bool) {
        let level = level.clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        if level > 1.0 {
            self.remembered = level;
        }
        if reduce_motion {
            self.level = still(level);
        } else {
            self.level.retarget(level);
        }
    }

    pub(crate) fn configure(&mut self, level: f64, reduce_motion: bool) {
        if let Some(index) = self.written.iter().position(|written| *written == level) {
            self.written.drain(..=index);
            return;
        }
        self.written.clear();
        if level != self.level.target() {
            self.go_to(level, reduce_motion);
        }
    }

    pub(crate) fn advance(&mut self, time: Duration) {
        let step = self
            .last
            .map_or(Duration::ZERO, |last| time.saturating_sub(last));
        self.last = Some(time);
        self.level.advance(step.as_secs_f64());
        if !self.is_animating() {
            self.level = still(self.level.target());
            self.last = None;
        }
    }

    pub(crate) fn follow_pointer(&mut self) {
        self.focus = None;
    }

    pub(crate) fn follow(&mut self, point: Point<f64, Logical>) {
        self.focus = Some(point);
    }
}

pub(crate) fn stepped(level: f64, closer: bool) -> f64 {
    let next = if closer { level * STEP } else { level / STEP };
    if next < STEP.sqrt() { 1.0 } else { next }
}

impl State {
    pub(crate) fn toggle_magnifier(&mut self) {
        let target = if self.zoom.target() > 1.0 {
            1.0
        } else {
            self.zoom.remembered
        };
        self.go_to_zoom(target);
    }

    pub(crate) fn step_magnifier(&mut self, closer: bool) {
        let target = stepped(self.zoom.target(), closer);
        self.go_to_zoom(target);
    }

    fn go_to_zoom(&mut self, level: f64) {
        self.zoom.go_to(level, self.reduce_motion);
        let level = self.zoom.target();
        self.zoom.written.push(level);
        self.remember_setting("Zoom", Value::from(level));
        self.needs_redraw = true;
    }

    fn zoom_follows(&self) -> Point<f64, Logical> {
        self.zoom.focus.unwrap_or(self.pointer_location)
    }

    pub(crate) fn zoom_on(&self, screen: usize) -> f64 {
        let level = self.zoom.level();
        let followed = self
            .screen_at(self.zoom_follows())
            .unwrap_or_else(|| self.pointer_screen());
        if level > 1.0 && followed == screen && self.screens[screen].mirror.is_none() {
            level
        } else {
            1.0
        }
    }

    pub(crate) fn render_scale(&self, screen: usize) -> f64 {
        self.screens[screen].scale() * self.zoom_on(screen)
    }

    pub(crate) fn view_origin_in_pixels(&self, screen: usize, scale: f64) -> Point<i32, Physical> {
        let rect = self.screen_rect(screen);
        let origin = rect.location.to_f64();
        let level = self.zoom_on(screen);
        if level <= 1.0 {
            return origin.to_physical(scale).to_i32_round();
        }
        let size = rect.size.to_f64();
        let followed = self.zoom_follows();
        let inside = Point::<f64, Logical>::from((
            (followed.x - origin.x).clamp(0.0, size.w),
            (followed.y - origin.y).clamp(0.0, size.h),
        ));
        let pan = 1.0 - 1.0 / level;
        let view = origin + Point::from((inside.x * pan, inside.y * pan));
        view.to_physical(scale).to_i32_round()
    }

    pub(crate) fn animate_zoom(&mut self, time: Duration) {
        if self.zoom.is_animating() {
            self.zoom.advance(time);
        }
    }
}

impl Headless {
    pub fn zoom_level(&self) -> f64 {
        self.state.zoom.level()
    }

    pub fn render_scale(&self, output: &str) -> Option<f64> {
        Some(self.state.render_scale(self.state.screen_named(output)?))
    }
}
