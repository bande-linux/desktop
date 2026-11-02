use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Node, Role};
use bande_graphics::{Color, Item, Point, Rect, Scroll, ScrollPhase, ScrollSource, Shape};
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
const SCROLLBAR_HOVER_WIDTH: f32 = 10.0;
const SCROLLBAR_INSET: f32 = 3.0;
const TRACK_OPACITY: f32 = 0.25;
const KNOB_MIN: f32 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollExtent {
    pub offset: f32,
    pub content: f32,
    pub viewport: f32,
}

impl ScrollExtent {
    pub fn limit(&self) -> f32 {
        (self.content - self.viewport).max(0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollbarAction {
    Redraw,
    Page(f32),
    Seek(f32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Visibility {
    Hidden,
    Shown { until: Instant },
    Fading { since: Instant },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scrollbar {
    visibility: Visibility,
    hovered: bool,
    width: Animated,
    grab: Option<f32>,
}

impl Default for Scrollbar {
    fn default() -> Self {
        Self {
            visibility: Visibility::Hidden,
            hovered: false,
            width: Animated::points(SCROLLBAR_WIDTH),
            grab: None,
        }
    }
}

impl Scrollbar {
    pub fn reveal(&mut self, now: Instant) {
        self.visibility = Visibility::Shown {
            until: now + SCROLLBAR_LINGER,
        };
    }

    fn held(&self) -> bool {
        self.hovered || self.grab.is_some()
    }

    fn fade(&self, now: Instant) -> (f32, bool) {
        if self.held() {
            return (1.0, false);
        }
        let since = match self.visibility {
            Visibility::Hidden => return (0.0, false),
            Visibility::Shown { until } if now <= until => return (1.0, false),
            Visibility::Shown { until } => until,
            Visibility::Fading { since } => since,
        };
        let mut fade = Animated::new(1.0);
        fade.animate_at(since, false, 0.0, Spring::FADE);
        fade.value_at(now)
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.fade(now).1 || self.width.value_at(now).1
    }

    pub fn deadline(&self) -> Option<Instant> {
        match self.visibility {
            Visibility::Shown { until } if !self.held() => Some(until),
            _ => None,
        }
    }

    pub fn tick(&mut self, now: Instant) {
        if let Visibility::Shown { until } = self.visibility
            && until <= now
        {
            self.visibility = Visibility::Fading { since: until };
        }
    }

    fn strip(frame: Rect) -> Rect {
        Rect::new(
            frame.x + frame.width - SCROLLBAR_STRIP,
            frame.y,
            SCROLLBAR_STRIP,
            frame.height,
        )
    }

    fn knob(&self, now: Instant, frame: Rect, extent: &ScrollExtent) -> Rect {
        let (width, _) = self.width.value_at(now);
        let limit = extent.limit();
        let track = frame.height - 2.0 * SCROLLBAR_INSET;
        let length =
            (track * extent.viewport / extent.content.max(1.0)).clamp(KNOB_MIN.min(track), track);
        let overshoot = if extent.offset < 0.0 {
            -extent.offset
        } else {
            (extent.offset - limit).max(0.0)
        };
        let length = (length - overshoot).max(width);
        let progress = (extent.offset / limit.max(1.0)).clamp(0.0, 1.0);
        Rect::new(
            frame.x + frame.width - SCROLLBAR_INSET - width,
            frame.y + SCROLLBAR_INSET + progress * (track - length),
            width,
            length,
        )
    }

    pub fn claims(&self, frame: Rect, extent: &ScrollExtent, point: Point) -> bool {
        extent.limit() > 0.0 && Self::strip(frame).contains(point)
    }

    fn hover(&mut self, now: Instant, reduce_motion: bool, hovered: bool) -> bool {
        if hovered == self.hovered {
            return false;
        }
        self.hovered = hovered;
        self.reveal(now);
        let width = if hovered {
            SCROLLBAR_HOVER_WIDTH
        } else {
            SCROLLBAR_WIDTH
        };
        self.width
            .animate_at(now, reduce_motion, width, Spring::QUICK);
        true
    }

    pub fn pointer(
        &mut self,
        now: Instant,
        reduce_motion: bool,
        frame: Rect,
        extent: &ScrollExtent,
        event: &Event,
    ) -> Option<ScrollbarAction> {
        match event {
            Event::PointerDown(pointer)
                if pointer.is_primary() && self.claims(frame, extent, pointer.position) =>
            {
                self.reveal(now);
                let knob = self.knob(now, frame, extent);
                if (knob.y..knob.y + knob.height).contains(&pointer.position.y) {
                    self.grab = Some(pointer.position.y - knob.y);
                    return Some(ScrollbarAction::Redraw);
                }
                let direction = if pointer.position.y < knob.y {
                    -1.0
                } else {
                    1.0
                };
                Some(ScrollbarAction::Page(direction))
            }
            Event::PointerMove(pointer) | Event::PointerEnter(pointer) => {
                if let Some(grab) = self.grab {
                    let knob = self.knob(now, frame, extent);
                    let track = frame.height - 2.0 * SCROLLBAR_INSET - knob.height;
                    let progress =
                        (pointer.position.y - grab - frame.y - SCROLLBAR_INSET) / track.max(1.0);
                    return Some(ScrollbarAction::Seek(
                        progress.clamp(0.0, 1.0) * extent.limit(),
                    ));
                }
                let hovered = self.claims(frame, extent, pointer.position);
                self.hover(now, reduce_motion, hovered)
                    .then_some(ScrollbarAction::Redraw)
            }
            Event::PointerUp(pointer) => {
                self.grab.take()?;
                let hovered = self.claims(frame, extent, pointer.position);
                self.hover(now, reduce_motion, hovered);
                self.reveal(now);
                Some(ScrollbarAction::Redraw)
            }
            Event::PointerLeave => {
                let grabbed = self.grab.take().is_some();
                let left = self.hover(now, reduce_motion, false);
                (grabbed || left).then_some(ScrollbarAction::Redraw)
            }
            _ => None,
        }
    }

    pub fn paint(
        &self,
        now: Instant,
        frame: Rect,
        extent: &ScrollExtent,
        color: Color,
        items: &mut Vec<Item>,
    ) {
        let (alpha, _) = self.fade(now);
        if extent.limit() == 0.0 || alpha == 0.0 {
            return;
        }
        let knob = self.knob(now, frame, extent);
        let expanded = (knob.width - SCROLLBAR_WIDTH) / (SCROLLBAR_HOVER_WIDTH - SCROLLBAR_WIDTH);
        if expanded > 0.0 {
            let lane = knob.x - SCROLLBAR_INSET;
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    lane,
                    frame.y,
                    frame.x + frame.width - lane,
                    frame.height,
                )),
                paint: color
                    .with_opacity(alpha * expanded.min(1.0) * TRACK_OPACITY)
                    .into(),
            });
        }
        items.push(Item::Fill {
            shape: Shape::rounded(knob, knob.width / 2.0),
            paint: color.with_opacity(alpha).into(),
        });
    }
}

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
    scrollbar: Scrollbar,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self {
            motion: Motion::Still(0.0),
            content: 0.0,
            viewport: 0.0,
            samples: Vec::new(),
            scrollbar: Scrollbar::default(),
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
        if moving || self.scrollbar.animating(cx.now()) {
            cx.animating.set(true);
        }
        if let Some(until) = self.scrollbar.deadline().filter(|until| *until >= cx.now()) {
            cx.wake_at(until);
        }
        offset
    }

    fn extent(&self, now: Instant) -> ScrollExtent {
        ScrollExtent {
            offset: self.offset_at(now).0,
            content: self.content,
            viewport: self.viewport,
        }
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
        self.scrollbar.reveal(now);
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
        self.scrollbar.reveal(now);
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

    pub fn claims(&self, frame: Rect, point: Point) -> bool {
        let extent = ScrollExtent {
            offset: 0.0,
            content: self.content,
            viewport: self.viewport,
        };
        self.scrollbar.claims(frame, &extent, point)
    }

    pub fn pointer(&mut self, cx: &Context, frame: Rect, event: &Event) -> bool {
        let now = cx.now();
        if let Event::PointerDown(pointer) = event
            && self.claims(frame, pointer.position)
        {
            self.settle(now);
        }
        let extent = self.extent(now);
        match self
            .scrollbar
            .pointer(now, cx.reduce_motion, frame, &extent, event)
        {
            None => false,
            Some(ScrollbarAction::Redraw) => true,
            Some(ScrollbarAction::Page(direction)) => {
                self.scroll_to(cx, extent.offset + direction * self.viewport);
                true
            }
            Some(ScrollbarAction::Seek(offset)) => {
                self.motion = Motion::Still(offset);
                true
            }
        }
    }

    pub fn paint_scrollbar(&self, cx: &Context, frame: Rect, items: &mut Vec<Item>) {
        let now = cx.now();
        self.scrollbar
            .paint(now, frame, &self.extent(now), cx.palette.ink3, items);
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
