use std::f32::consts::TAU;
use std::time::{Duration, Instant};

use accesskit::{Node, Role};
use unbound_graphics::{Item, Point, Rect, Shape};

use super::{Animated, Context, Size, Spring, Widget, WidgetId};

pub const PROGRESS_HEIGHT: f32 = 6.0;
pub const SPINNER_SIZE: f32 = 16.0;
const SWEEP: Duration = Duration::from_millis(1400);
const SWEEP_FRACTION: f32 = 0.3;
const SPOKES: u32 = 12;
const SPIN: Duration = Duration::from_millis(1000);

pub struct ProgressBar {
    id: WidgetId,
    label: String,
    value: Option<f32>,
    shown: Animated,
    started: Option<Instant>,
    frame: Rect,
}

impl ProgressBar {
    pub fn new(label: &str, value: Option<f32>) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            value: value.map(|value| value.clamp(0.0, 1.0)),
            shown: Animated::new(value.unwrap_or_default().clamp(0.0, 1.0)),
            started: None,
            frame: Rect::default(),
        }
    }

    pub fn value(&self) -> Option<f32> {
        self.value
    }

    pub fn set_value(&mut self, cx: &Context, value: Option<f32>) {
        let value = value.map(|value| value.clamp(0.0, 1.0));
        match (self.value, value) {
            (None, Some(value)) => self.shown.jump_to(value),
            (_, Some(value)) => self.shown.animate_to(cx, value, Spring::QUICK),
            _ => {}
        }
        self.value = value;
    }

    fn sweep(&self, cx: &Context, track: Rect, items: &mut Vec<Item>) {
        let elapsed = self.started.map_or(0.0, |started| {
            cx.now().saturating_duration_since(started).as_secs_f32()
        });
        let phase = (elapsed / SWEEP.as_secs_f32()).fract();
        cx.animating.set(true);
        if cx.reduce_motion {
            let alpha = 0.35 + 0.35 * (1.0 - (phase * TAU).cos()) / 2.0;
            items.push(Item::Fill {
                shape: Shape::rounded(track, track.height / 2.0),
                paint: cx
                    .palette
                    .accent_color
                    .with_alpha((alpha * 255.0) as u8)
                    .into(),
            });
            return;
        }
        let eased = (1.0 - (phase * std::f32::consts::PI).cos()) / 2.0;
        let width = track.width * SWEEP_FRACTION;
        let x = track.x - width + (track.width + width) * eased;
        items.push(Item::Clip {
            shape: Shape::rounded(track, track.height / 2.0),
            items: vec![Item::Fill {
                shape: Shape::rounded(
                    Rect::new(x, track.y, width, track.height),
                    track.height / 2.0,
                ),
                paint: cx.palette.accent_color.into(),
            }],
        });
    }
}

impl Widget for ProgressBar {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        if self.value.is_none() {
            self.started.get_or_insert(cx.now());
        }
        Size::new(available.width, PROGRESS_HEIGHT)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = Rect::new(frame.x, frame.y, frame.width, PROGRESS_HEIGHT);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let radius = self.frame.height / 2.0;
        items.push(Item::Fill {
            shape: Shape::rounded(self.frame, radius),
            paint: cx.palette.separator_color.into(),
        });
        if self.value.is_none() {
            self.sweep(cx, self.frame, items);
            return;
        }
        let width = self.frame.width * self.shown.value(cx);
        if width > 0.0 {
            items.push(Item::Fill {
                shape: Shape::rounded(
                    Rect::new(
                        self.frame.x,
                        self.frame.y,
                        width.max(self.frame.height),
                        self.frame.height,
                    ),
                    radius,
                ),
                paint: cx.palette.accent_color.into(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::ProgressIndicator);
        node.set_label(self.label.clone());
        match self.value {
            Some(value) => {
                node.set_numeric_value(f64::from(value) * 100.0);
                node.set_min_numeric_value(0.0);
                node.set_max_numeric_value(100.0);
            }
            None => node.set_busy(),
        }
        node
    }
}

pub struct Spinner {
    id: WidgetId,
    label: String,
    started: Option<Instant>,
    frame: Rect,
}

impl Spinner {
    pub fn new(label: &str) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            started: None,
            frame: Rect::default(),
        }
    }
}

impl Widget for Spinner {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        self.started.get_or_insert(cx.now());
        Size::new(SPINNER_SIZE, SPINNER_SIZE)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = Rect::new(frame.x, frame.y, SPINNER_SIZE, SPINNER_SIZE);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        cx.animating.set(true);
        let elapsed = self.started.map_or(0.0, |started| {
            cx.now().saturating_duration_since(started).as_secs_f32()
        });
        let lead = ((elapsed / SPIN.as_secs_f32()).fract() * SPOKES as f32) as u32;
        let centre = Point::new(
            self.frame.x + self.frame.width / 2.0,
            self.frame.y + self.frame.height / 2.0,
        );
        let (inner, outer) = (self.frame.width * 0.24, self.frame.width * 0.46);
        for spoke in 0..SPOKES {
            let angle = spoke as f32 / SPOKES as f32 * TAU - TAU / 4.0;
            let (sin, cos) = angle.sin_cos();
            let behind = (lead + SPOKES - spoke) % SPOKES;
            let alpha = 1.0 - 0.75 * behind as f32 / (SPOKES - 1) as f32;
            items.push(Item::Line {
                from: Point::new(centre.x + cos * inner, centre.y + sin * inner),
                to: Point::new(centre.x + cos * outer, centre.y + sin * outer),
                width: self.frame.width * 0.1,
                paint: cx
                    .palette
                    .label_muted
                    .with_alpha((alpha * 255.0) as u8)
                    .into(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::ProgressIndicator);
        node.set_label(self.label.clone());
        node.set_busy();
        node
    }
}
