use std::collections::VecDeque;
use std::time::Duration;

use smithay::delegate_pointer_gestures;
use smithay::input::pointer::{
    GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent,
    GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER};
use unbound_spring::{Animation, Decay, Spring, State as Phase, rubber_band};

use crate::state::{Headless, State};

const DESKTOP_FINGERS: u32 = 4;
const SPACE_FINGERS: u32 = 3;
const AXIS_LOCK: f64 = 16.0;
const SPREAD_FOR_DESKTOP: f64 = 0.6;
const PINCH_FOR_LAUNCHER: f64 = 0.8;
const VELOCITY_WINDOW_MS: f64 = 100.0;
const MOMENTUM_SPEED: f64 = 2.0;
const SETTLED_PROGRESS: f64 = 1e-3;
const SETTLED_SPEED: f64 = 1e-2;
const SLIVER: f64 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    SwipeBegin {
        fingers: u32,
    },
    SwipeUpdate {
        delta: Point<f64, Logical>,
    },
    SwipeEnd {
        cancelled: bool,
    },
    PinchBegin {
        fingers: u32,
    },
    PinchUpdate {
        delta: Point<f64, Logical>,
        scale: f64,
        rotation: f64,
    },
    PinchEnd {
        cancelled: bool,
    },
    HoldBegin {
        fingers: u32,
    },
    HoldEnd {
        cancelled: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Route {
    Client,
    Desktop { was_shown: bool, scale: f64 },
    Undecided { delta: Point<f64, Logical> },
    Workspaces,
    Overview,
}

#[derive(Debug, Clone)]
pub(crate) struct Tracker {
    start: f64,
    raw: f64,
    last: f64,
    samples: VecDeque<(f64, f64)>,
}

impl Tracker {
    pub(crate) fn new(start: f64, time: u32, last: f64) -> Self {
        Self {
            start,
            raw: start,
            last,
            samples: VecDeque::from([(f64::from(time), start)]),
        }
    }

    pub(crate) fn start(&self) -> f64 {
        self.start
    }

    pub(crate) fn raw(&self) -> f64 {
        self.raw
    }

    pub(crate) fn track(&mut self, raw: f64, time: u32) {
        let now = f64::from(time);
        self.raw = raw;
        if self.samples.back().is_some_and(|(at, _)| *at == now) {
            self.samples.pop_back();
        }
        self.samples.push_back((now, raw));
        while self
            .samples
            .front()
            .is_some_and(|(at, _)| now - at > VELOCITY_WINDOW_MS)
        {
            self.samples.pop_front();
        }
    }

    pub(crate) fn position(&self) -> f64 {
        let inside = self.raw.clamp(0.0, self.last);
        inside + rubber_band(self.raw - inside, 1.0)
    }

    fn velocity(&self) -> f64 {
        let count = self.samples.len() as f64;
        if count < 2.0 {
            return 0.0;
        }
        let mean_time = self.samples.iter().map(|(at, _)| at).sum::<f64>() / count;
        let mean_value = self.samples.iter().map(|(_, value)| value).sum::<f64>() / count;
        let (covariance, variance) =
            self.samples
                .iter()
                .fold((0.0, 0.0), |(covariance, variance), (at, value)| {
                    let offset = at - mean_time;
                    (
                        covariance + offset * (value - mean_value),
                        variance + offset * offset,
                    )
                });
        if variance == 0.0 {
            return 0.0;
        }
        covariance / variance * 1000.0
    }

    pub(crate) fn release(&self) -> (f64, f64) {
        let velocity = self.velocity();
        let projected = Decay::NORMAL.project(Phase {
            position: self.position(),
            velocity,
        });
        let lowest = (self.start.round() - 1.0).max(0.0);
        let highest = (self.start.round() + 1.0).min(self.last);
        (projected.round().clamp(lowest, highest), velocity)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ShowDesktop {
    tracker: Option<Tracker>,
    settle: Option<Animation>,
    last: Option<Duration>,
    progress: f64,
    target: f64,
    focus_before: Option<WlSurface>,
}

impl ShowDesktop {
    pub(crate) fn hides_windows(&self) -> bool {
        self.progress != 0.0 || self.target != 0.0 || self.tracker.is_some()
    }

    pub(crate) fn is_animating(&self) -> bool {
        self.settle.is_some()
    }

    fn settle_to(&mut self, target: f64, velocity: f64, spring: Spring) {
        self.tracker = None;
        self.target = target;
        self.last = None;
        self.settle = Some(Animation::new(
            spring,
            Phase {
                position: self.progress,
                velocity,
            },
            target,
        ));
    }
}

fn shift_for(rect: Rectangle<f64, Logical>, left: f64, width: f64) -> f64 {
    if rect.loc.x + rect.size.w / 2.0 < left + width / 2.0 {
        left + SLIVER - (rect.loc.x + rect.size.w)
    } else {
        left + width - SLIVER - rect.loc.x
    }
}

impl State {
    pub(crate) fn desktop_shift(
        &self,
        rect: Rectangle<f64, Logical>,
        screen: usize,
    ) -> Point<f64, Logical> {
        let progress = self.desktop.progress;
        if progress == 0.0 {
            return Point::default();
        }
        let output = self.screen_rect(screen);
        let (left, width) = (f64::from(output.location.x), f64::from(output.size.w));
        (shift_for(rect, left, width) * progress, 0.0).into()
    }

    fn release_spring(&self, velocity: f64) -> Spring {
        if self.reduce_motion {
            Spring::FADE
        } else if velocity.abs() > MOMENTUM_SPEED {
            Spring::MOMENTUM
        } else {
            Spring::STANDARD
        }
    }

    pub(crate) fn hide_desktop(&mut self) {
        if self.desktop.target == 0.0 && self.desktop.tracker.is_none() {
            return;
        }
        let spring = self.release_spring(0.0);
        self.desktop.settle_to(0.0, 0.0, spring);
        self.desktop.focus_before = None;
        self.needs_redraw = true;
    }

    pub(crate) fn animate_desktop(&mut self, time: Duration) {
        let refresh = self.main_screen().refresh();

        let desktop = &mut self.desktop;
        let Some(settle) = desktop.settle.as_mut() else {
            return;
        };
        let step = desktop
            .last
            .map_or(refresh, |last| time.saturating_sub(last));
        desktop.last = Some(time);
        desktop.progress = settle.advance(step.as_secs_f64()).position;
        if settle.is_settled(SETTLED_PROGRESS, SETTLED_SPEED) {
            desktop.progress = desktop.target;
            desktop.settle = None;
            desktop.last = None;
        }
        self.needs_redraw = true;
    }

    fn begin_desktop(&mut self, time: u32) -> Route {
        let was_shown = self.desktop.target != 0.0;
        self.desktop.settle = None;
        self.desktop.last = None;
        self.desktop.tracker = Some(Tracker::new(self.desktop.progress, time, 1.0));
        self.refocus_pointer();
        Route::Desktop {
            was_shown,
            scale: 1.0,
        }
    }

    fn track_desktop(&mut self, scale: f64, time: u32) {
        let Some(tracker) = self.desktop.tracker.as_mut() else {
            return;
        };
        let raw = tracker.start + (scale - 1.0) / SPREAD_FOR_DESKTOP;
        tracker.track(raw, time);
        self.desktop.progress = tracker.position();
        self.needs_redraw = true;
    }

    fn end_desktop(&mut self, was_shown: bool, scale: f64, cancelled: bool) {
        let Some(tracker) = self.desktop.tracker.take() else {
            return;
        };
        let (target, velocity) = if cancelled {
            (tracker.start, 0.0)
        } else {
            tracker.release()
        };
        let spring = self.release_spring(velocity);
        self.desktop.settle_to(target, velocity, spring);
        if !cancelled && !was_shown && scale < PINCH_FOR_LAUNCHER {
            self.show_launcher();
        }
        if target != tracker.start {
            self.hand_keyboard_over(target);
        }
        self.needs_redraw = true;
        self.refocus_pointer();
    }

    fn hand_keyboard_over(&mut self, target: f64) {
        if target == 0.0 {
            let previous = self.desktop.focus_before.take();
            if previous.is_some() {
                self.focus(previous);
            }
            return;
        }
        self.desktop.focus_before = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus());
        self.focus(None);
    }

    fn decide_swipe(&mut self, delta: Point<f64, Logical>, time: u32) -> Route {
        if delta.x.hypot(delta.y) < AXIS_LOCK {
            return Route::Undecided { delta };
        }
        if delta.x.abs() >= delta.y.abs() {
            self.begin_workspace_swipe(time);
            self.track_workspace_swipe(delta.x, time);
            return Route::Workspaces;
        }
        self.begin_overview_swipe(time, delta.y < 0.0);
        self.track_overview_swipe(delta.y, time);
        Route::Overview
    }

    fn forward(&mut self, gesture: Gesture, time: u32) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        match gesture {
            Gesture::SwipeBegin { fingers } => pointer.gesture_swipe_begin(
                self,
                &GestureSwipeBeginEvent {
                    serial,
                    time,
                    fingers,
                },
            ),
            Gesture::SwipeUpdate { delta } => {
                pointer.gesture_swipe_update(self, &GestureSwipeUpdateEvent { time, delta });
            }
            Gesture::SwipeEnd { cancelled } => pointer.gesture_swipe_end(
                self,
                &GestureSwipeEndEvent {
                    serial,
                    time,
                    cancelled,
                },
            ),
            Gesture::PinchBegin { fingers } => pointer.gesture_pinch_begin(
                self,
                &GesturePinchBeginEvent {
                    serial,
                    time,
                    fingers,
                },
            ),
            Gesture::PinchUpdate {
                delta,
                scale,
                rotation,
            } => pointer.gesture_pinch_update(
                self,
                &GesturePinchUpdateEvent {
                    time,
                    delta,
                    scale,
                    rotation,
                },
            ),
            Gesture::PinchEnd { cancelled } => pointer.gesture_pinch_end(
                self,
                &GesturePinchEndEvent {
                    serial,
                    time,
                    cancelled,
                },
            ),
            Gesture::HoldBegin { fingers } => pointer.gesture_hold_begin(
                self,
                &GestureHoldBeginEvent {
                    serial,
                    time,
                    fingers,
                },
            ),
            Gesture::HoldEnd { cancelled } => pointer.gesture_hold_end(
                self,
                &GestureHoldEndEvent {
                    serial,
                    time,
                    cancelled,
                },
            ),
        }
        pointer.frame(self);
    }

    fn route_gesture(&mut self, gesture: Gesture, time: u32) {
        let route = match (gesture, self.gesture_route) {
            (Gesture::PinchBegin { fingers }, _)
                if fingers == DESKTOP_FINGERS && !self.is_locked() =>
            {
                Some(self.begin_desktop(time))
            }
            (Gesture::SwipeBegin { fingers }, _)
                if fingers == SPACE_FINGERS && !self.is_locked() =>
            {
                Some(Route::Undecided {
                    delta: Point::default(),
                })
            }
            (
                Gesture::SwipeBegin { .. } | Gesture::PinchBegin { .. } | Gesture::HoldBegin { .. },
                _,
            ) => Some(Route::Client),
            (Gesture::SwipeUpdate { delta }, Some(Route::Undecided { delta: before })) => {
                Some(self.decide_swipe(before + delta, time))
            }
            (Gesture::SwipeUpdate { delta }, Some(Route::Workspaces)) => {
                self.track_workspace_swipe(delta.x, time);
                Some(Route::Workspaces)
            }
            (Gesture::SwipeEnd { cancelled }, Some(Route::Workspaces)) => {
                self.end_workspace_swipe(cancelled);
                None
            }
            (Gesture::SwipeUpdate { delta }, Some(Route::Overview)) => {
                self.track_overview_swipe(delta.y, time);
                Some(Route::Overview)
            }
            (Gesture::SwipeEnd { cancelled }, Some(Route::Overview)) => {
                self.end_overview_swipe(cancelled);
                None
            }
            (Gesture::PinchUpdate { scale, .. }, Some(Route::Desktop { was_shown, .. })) => {
                self.track_desktop(scale, time);
                Some(Route::Desktop { was_shown, scale })
            }
            (Gesture::PinchEnd { cancelled }, Some(Route::Desktop { was_shown, scale })) => {
                self.end_desktop(was_shown, scale, cancelled);
                None
            }
            (_, route) => route,
        };
        if route == Some(Route::Client) || self.gesture_route == Some(Route::Client) {
            self.forward(gesture, time);
        }
        self.gesture_route = match gesture {
            Gesture::SwipeEnd { .. } | Gesture::PinchEnd { .. } | Gesture::HoldEnd { .. } => None,
            _ => route,
        };
    }
}

impl Headless {
    pub fn gesture(&mut self, gesture: Gesture, time: u32) {
        self.state.note_activity();
        self.state.route_gesture(gesture, time);
    }

    pub fn shows_desktop(&self) -> bool {
        self.state.desktop.target != 0.0
    }
}

delegate_pointer_gestures!(State);
