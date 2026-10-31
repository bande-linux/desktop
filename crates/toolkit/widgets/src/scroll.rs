use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Node, Role};
use bande_graphics::{Item, Point, Rect, Scroll, ScrollPhase, ScrollSource, Shape};
use bande_spring::Spring;

use super::{Animated, Context, Event, Size, Widget, WidgetId};

const FLING_TIME_CONSTANT: f32 = 0.325;
const FLING_MIN_SPEED: f32 = 60.0;
const FLING_STOP_SPEED: f32 = 8.0;
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);
const RUBBER_BAND: f32 = 0.55;
const SCROLLBAR_LINGER: Duration = Duration::from_millis(900);
const SCROLLBAR_STRIP: f32 = 14.0;
const SCROLLBAR_WIDTH: f32 = 6.0;
const SCROLLBAR_INSET: f32 = 3.0;
const KNOB_MIN: f32 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Motion {
    Still(f32),
    Tracking(f32),
    Spring(Animated),
    Fling {
        since: Instant,
        from: f32,
        velocity: f32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScrollState {
    motion: Motion,
    content: f32,
    viewport: f32,
    samples: Vec<(Duration, f32)>,
    shown_until: Option<Instant>,
    knob_grab: Option<f32>,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self {
            motion: Motion::Still(0.0),
            content: 0.0,
            viewport: 0.0,
            samples: Vec::new(),
            shown_until: None,
            knob_grab: None,
        }
    }
}

impl ScrollState {
    pub fn limit(&self) -> f32 {
        (self.content - self.viewport).max(0.0)
    }

    pub fn set_extent(&mut self, content: f32, viewport: f32) {
        self.content = content;
        self.viewport = viewport;
        match self.motion {
            Motion::Still(offset) => self.motion = Motion::Still(offset.clamp(0.0, self.limit())),
            Motion::Spring(animated) if animated.target() > self.limit() => {
                self.motion = Motion::Still(self.limit());
            }
            _ => {}
        }
    }

    fn rubber(&self, raw: f32) -> f32 {
        let band = |over: f32| {
            let dimension = self.viewport.max(1.0);
            (1.0 - 1.0 / (over * RUBBER_BAND / dimension + 1.0)) * dimension
        };
        if raw < 0.0 {
            -band(-raw)
        } else if raw > self.limit() {
            self.limit() + band(raw - self.limit())
        } else {
            raw
        }
    }

    fn offset_at(&self, now: Instant) -> (f32, bool) {
        match self.motion {
            Motion::Still(offset) => (offset, false),
            Motion::Tracking(raw) => (self.rubber(raw), false),
            Motion::Spring(animated) => animated.value_at(now),
            Motion::Fling {
                since,
                from,
                velocity,
            } => {
                let stop = FLING_TIME_CONSTANT * (velocity.abs() / FLING_STOP_SPEED).ln().max(0.0);
                let elapsed = now.saturating_duration_since(since).as_secs_f32();
                let decay = (-elapsed.min(stop) / FLING_TIME_CONSTANT).exp();
                let position = from + velocity * FLING_TIME_CONSTANT * (1.0 - decay);
                let clamped = position.clamp(0.0, self.limit());
                (clamped, elapsed < stop && clamped == position)
            }
        }
    }

    pub fn offset(&self, cx: &Context) -> f32 {
        let (offset, moving) = self.offset_at(cx.now());
        if moving || self.scrollbar_fading(cx.now()) {
            cx.animating.set(true);
        }
        if let Some(until) = self.shown_until.filter(|until| *until >= cx.now()) {
            cx.wake_at(until);
        }
        offset
    }

    fn target(&self, now: Instant) -> f32 {
        match self.motion {
            Motion::Spring(animated) => animated.target(),
            _ => self.offset_at(now).0,
        }
    }

    fn settle(&mut self, now: Instant) {
        let (offset, _) = self.offset_at(now);
        self.motion = Motion::Still(offset.clamp(0.0, self.limit()));
    }

    pub fn scroll_to(&mut self, cx: &Context, target: f32) -> bool {
        let now = cx.now();
        let target = target.clamp(0.0, self.limit());
        if target == self.target(now) {
            return false;
        }
        let (current, _) = self.offset_at(now);
        let mut animated = match self.motion {
            Motion::Spring(animated) => animated,
            _ => Animated::points(current),
        };
        animated.animate_at(now, cx.reduce_motion, target, Spring::QUICK);
        self.motion = Motion::Spring(animated);
        self.shown_until = Some(now + SCROLLBAR_LINGER);
        cx.animating.set(true);
        true
    }

    fn jump_to(&mut self, offset: f32) {
        self.motion = Motion::Still(offset.clamp(0.0, self.limit()));
    }

    pub fn follow(&mut self, cx: &Context, top: f32, bottom: f32) -> bool {
        let current = self.offset_at(cx.now()).0;
        let target = if top < current {
            top
        } else if bottom > current + self.viewport {
            bottom - self.viewport
        } else {
            return false;
        };
        self.jump_to(target);
        true
    }

    pub fn reveal(&mut self, cx: &Context, top: f32, bottom: f32) -> bool {
        let target = self.target(cx.now());
        if top < target {
            self.scroll_to(cx, top)
        } else if bottom > target + self.viewport {
            self.scroll_to(cx, bottom - self.viewport)
        } else {
            false
        }
    }

    pub fn scroll(&mut self, cx: &Context, scroll: &Scroll) -> bool {
        let now = cx.now();
        let tracking = matches!(self.motion, Motion::Tracking(_));
        if self.limit() == 0.0 && !tracking {
            return false;
        }
        let delta = scroll.delta.y;
        self.shown_until = Some(now + SCROLLBAR_LINGER);
        cx.animating.set(true);
        if scroll.source == ScrollSource::Wheel {
            let target = self.target(now) + delta;
            return self.scroll_to(cx, target) || self.limit() > 0.0;
        }
        match scroll.phase {
            ScrollPhase::Began => {
                let (current, _) = self.offset_at(now);
                self.motion = Motion::Tracking(current + delta);
                self.samples.clear();
                self.samples.push((scroll.time, delta));
            }
            ScrollPhase::Changed => {
                let raw = match self.motion {
                    Motion::Tracking(raw) => raw,
                    _ => self.offset_at(now).0,
                };
                self.motion = Motion::Tracking(raw + delta);
                self.samples.push((scroll.time, delta));
                self.forget_samples_before(scroll.time);
            }
            ScrollPhase::Ended => self.release(cx, scroll.time),
        }
        true
    }

    fn forget_samples_before(&mut self, time: Duration) {
        self.samples
            .retain(|(at, _)| time.saturating_sub(*at) <= VELOCITY_WINDOW);
    }

    fn release_velocity(&mut self, time: Duration) -> f32 {
        self.forget_samples_before(time);
        let samples = std::mem::take(&mut self.samples);
        let (Some((first, _)), Some((last, _))) = (samples.first(), samples.last()) else {
            return 0.0;
        };
        let span = last.saturating_sub(*first).as_secs_f32();
        if span <= 0.0 {
            return 0.0;
        }
        let travelled: f32 = samples[1..].iter().map(|(_, delta)| delta).sum();
        travelled / span
    }

    fn release(&mut self, cx: &Context, time: Duration) {
        let now = cx.now();
        let Motion::Tracking(raw) = self.motion else {
            return;
        };
        let velocity = self.release_velocity(time);
        if raw < 0.0 || raw > self.limit() {
            let mut animated = Animated::points(self.rubber(raw));
            animated.animate_at(
                now,
                cx.reduce_motion,
                raw.clamp(0.0, self.limit()),
                Spring::QUICK,
            );
            self.motion = Motion::Spring(animated);
        } else if velocity.abs() > FLING_MIN_SPEED && !cx.reduce_motion {
            self.motion = Motion::Fling {
                since: now,
                from: raw,
                velocity,
            };
        } else {
            self.motion = Motion::Still(raw);
        }
    }

    fn scrollbar_fade(&self, now: Instant) -> (f32, bool) {
        if self.knob_grab.is_some() {
            return (1.0, false);
        }
        let Some(until) = self.shown_until else {
            return (0.0, false);
        };
        if now <= until {
            return (1.0, false);
        }
        let mut fade = Animated::new(1.0);
        fade.animate_at(until, false, 0.0, Spring::FADE);
        fade.value_at(now)
    }

    fn scrollbar_alpha(&self, now: Instant) -> f32 {
        self.scrollbar_fade(now).0
    }

    fn scrollbar_fading(&self, now: Instant) -> bool {
        self.scrollbar_fade(now).1
    }

    fn strip(frame: Rect) -> Rect {
        Rect::new(
            frame.x + frame.width - SCROLLBAR_STRIP,
            frame.y,
            SCROLLBAR_STRIP,
            frame.height,
        )
    }

    fn knob(&self, frame: Rect, offset: f32) -> Rect {
        let track = frame.height - 2.0 * SCROLLBAR_INSET;
        let length =
            (track * self.viewport / self.content.max(1.0)).clamp(KNOB_MIN.min(track), track);
        let overshoot = if offset < 0.0 {
            -offset
        } else {
            (offset - self.limit()).max(0.0)
        };
        let length = (length - overshoot).max(SCROLLBAR_WIDTH);
        let progress = (offset / self.limit().max(1.0)).clamp(0.0, 1.0);
        Rect::new(
            frame.x + frame.width - SCROLLBAR_INSET - SCROLLBAR_WIDTH,
            frame.y + SCROLLBAR_INSET + progress * (track - length),
            SCROLLBAR_WIDTH,
            length,
        )
    }

    pub fn claims(&self, frame: Rect, point: Point) -> bool {
        self.limit() > 0.0 && Self::strip(frame).contains(point)
    }

    pub fn pointer(&mut self, cx: &Context, frame: Rect, event: &Event) -> bool {
        let now = cx.now();
        match event {
            Event::PointerDown(pointer)
                if pointer.is_primary() && self.claims(frame, pointer.position) =>
            {
                self.settle(now);
                let (offset, _) = self.offset_at(now);
                let knob = self.knob(frame, offset);
                if !(knob.y..knob.y + knob.height).contains(&pointer.position.y) {
                    let page = if pointer.position.y < knob.y {
                        -self.viewport
                    } else {
                        self.viewport
                    };
                    self.scroll_to(cx, offset + page);
                    return true;
                }
                self.knob_grab = Some(pointer.position.y - knob.y);
                self.shown_until = Some(now + SCROLLBAR_LINGER);
                true
            }
            Event::PointerMove(pointer) => {
                if let Some(grab) = self.knob_grab {
                    let (offset, _) = self.offset_at(now);
                    let knob = self.knob(frame, offset);
                    let track = frame.height - 2.0 * SCROLLBAR_INSET - knob.height;
                    let progress =
                        (pointer.position.y - grab - frame.y - SCROLLBAR_INSET) / track.max(1.0);
                    self.motion = Motion::Still(progress.clamp(0.0, 1.0) * self.limit());
                    self.shown_until = Some(now + SCROLLBAR_LINGER);
                    return true;
                }
                if self.claims(frame, pointer.position) {
                    let was_shown = self.scrollbar_alpha(now) > 0.0;
                    self.shown_until = Some(now + SCROLLBAR_LINGER);
                    cx.animating.set(true);
                    return !was_shown;
                }
                false
            }
            Event::PointerUp(_) => {
                let grabbed = self.knob_grab.take().is_some();
                if grabbed {
                    self.shown_until = Some(now + SCROLLBAR_LINGER);
                }
                grabbed
            }
            _ => false,
        }
    }

    pub fn paint_scrollbar(&self, cx: &Context, frame: Rect, items: &mut Vec<Item>) {
        let alpha = self.scrollbar_alpha(cx.now());
        if self.limit() == 0.0 || alpha == 0.0 {
            return;
        }
        let (offset, _) = self.offset_at(cx.now());
        let knob = self.knob(frame, offset);
        items.push(Item::Fill {
            shape: Shape::rounded(knob, SCROLLBAR_WIDTH / 2.0),
            paint: cx.palette.ink3.with_opacity(alpha).into(),
        });
    }

    pub fn describe(&self, cx: &Context, node: &mut Node) {
        node.set_scroll_y(f64::from(self.offset_at(cx.now()).0));
        node.set_scroll_y_min(0.0);
        node.set_scroll_y_max(f64::from(self.limit()));
        if self.limit() > 0.0 {
            node.add_action(Action::ScrollUp);
            node.add_action(Action::ScrollDown);
        }
    }

    pub fn perform(&mut self, cx: &Context, request: &ActionRequest) -> bool {
        let target = self.target(cx.now());
        match request.action {
            Action::ScrollUp => self.scroll_to(cx, target - self.viewport),
            Action::ScrollDown => self.scroll_to(cx, target + self.viewport),
            _ => false,
        }
    }
}

pub struct ScrollView {
    id: WidgetId,
    child: [Box<dyn Widget>; 1],
    state: Rc<RefCell<ScrollState>>,
    content: Size,
    frame: Rect,
}

impl ScrollView {
    pub fn new(child: impl Widget + 'static) -> Self {
        Self::shared(Box::new(child), Rc::default())
    }

    pub fn shared(child: Box<dyn Widget>, state: Rc<RefCell<ScrollState>>) -> Self {
        Self {
            id: WidgetId::next(),
            child: [child],
            state,
            content: Size::default(),
            frame: Rect::default(),
        }
    }
}

impl Widget for ScrollView {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.content = self.child[0].measure(cx, available);
        Size::new(available.width, self.content.height.min(available.height))
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let offset = {
            let mut state = self.state.borrow_mut();
            state.set_extent(self.content.height, frame.height);
            state.offset(cx)
        };
        self.child[0].place(
            cx,
            Rect::new(
                frame.x,
                frame.y - offset,
                frame.width,
                self.content.height.max(frame.height),
            ),
        );
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let mut inner = Vec::new();
        self.child[0].paint(cx, &mut inner);
        items.push(Item::Clip {
            shape: Shape::rect(self.frame),
            items: inner,
        });
        self.state.borrow().paint_scrollbar(cx, self.frame, items);
    }

    fn access(&self, cx: &Context) -> Node {
        let mut node = Node::new(Role::ScrollView);
        self.state.borrow().describe(cx, &mut node);
        node
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        let mut state = self.state.borrow_mut();
        match event {
            Event::Scroll(scroll) => state.scroll(cx, scroll),
            _ => state.pointer(cx, self.frame, event),
        }
    }

    fn perform(&mut self, cx: &mut Context, request: &ActionRequest) -> bool {
        self.state.borrow_mut().perform(cx, request)
    }

    fn hit_children(&self, point: Point) -> bool {
        !self.state.borrow().claims(self.frame, point)
    }

    fn reveal(&mut self, cx: &mut Context, rect: Rect) -> bool {
        let mut state = self.state.borrow_mut();
        let top = rect.y - self.frame.y + state.offset(cx);
        state.reveal(cx, top, top + rect.height)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.child
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.child
    }
}
