use accesskit::{Action, ActionData, ActionRequest, Node, Role};

use super::control::outline;
use super::{Animated, Context, Event, Key, Size, Spring, Widget, WidgetId};
use unbound_graphics::{Color, Item, Point, Rect, Shape};

pub const SLIDER_HEIGHT: f32 = 20.0;
const TRACK_HEIGHT: f32 = 4.0;
const KNOB_WIDTH: f32 = 7.0;
const KNOB_RADIUS: f32 = 2.0;
const KNOB_SHADOW: f32 = 0.2;
const TICK_SIZE: (f32, f32) = (1.5, 4.0);
const TICK_GAP: f32 = 3.0;

pub struct Slider {
    id: WidgetId,
    label: String,
    value: f64,
    range: (f64, f64),
    step: f64,
    on_change: Box<dyn FnMut(f64)>,
    dragging: bool,
    ticks: usize,
    knob: Animated,
    frame: Rect,
}

fn fraction(value: f64, range: (f64, f64)) -> f32 {
    let span = range.1 - range.0;
    match span > 0.0 {
        true => ((value - range.0) / span) as f32,
        false => 0.0,
    }
}

impl Slider {
    pub fn new(
        label: &str,
        value: f64,
        range: (f64, f64),
        step: f64,
        on_change: impl FnMut(f64) + 'static,
    ) -> Self {
        let value = value.clamp(range.0, range.1);
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            value,
            range,
            step,
            on_change: Box::new(on_change),
            dragging: false,
            ticks: 0,
            knob: Animated::new(fraction(value, range)),
            frame: Rect::default(),
        }
    }

    pub fn ticks(self, ticks: usize) -> Self {
        Self { ticks, ..self }
    }

    pub fn value(&self) -> f64 {
        self.value
    }

    pub fn set_value(&mut self, cx: &Context, value: f64) {
        if self.dragging {
            return;
        }
        self.value = value.clamp(self.range.0, self.range.1);
        cx.animate(
            &mut self.knob,
            fraction(self.value, self.range),
            Spring::INTERACTIVE,
        );
    }

    fn has_detents(&self) -> bool {
        self.ticks >= 2
    }

    fn set(&mut self, cx: &Context, value: f64, glide: bool) {
        let value = self.detent(value.clamp(self.range.0, self.range.1));
        let shown = fraction(value, self.range);
        match glide {
            true => cx.animate(&mut self.knob, shown, Spring::INTERACTIVE),
            false => self.knob.jump_to(shown),
        }
        if value != self.value {
            self.value = value;
            (self.on_change)(value);
        }
    }

    fn interval(&self) -> f64 {
        match self.has_detents() {
            true => (self.range.1 - self.range.0) / (self.ticks - 1) as f64,
            false => self.step,
        }
    }

    fn detent(&self, value: f64) -> f64 {
        if !self.has_detents() {
            return value;
        }
        let interval = self.interval();
        self.range.0 + ((value - self.range.0) / interval).round() * interval
    }

    fn track_x(&self) -> (f32, f32) {
        (
            self.frame.x + KNOB_WIDTH / 2.0,
            (self.frame.width - KNOB_WIDTH).max(0.0),
        )
    }

    fn value_at(&self, point: Point) -> f64 {
        let (start, length) = self.track_x();
        let fraction = if length > 0.0 {
            f64::from(((point.x - start) / length).clamp(0.0, 1.0))
        } else {
            0.0
        };
        self.range.0 + fraction * (self.range.1 - self.range.0)
    }
}

impl Widget for Slider {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        Size::new(available.width, SLIDER_HEIGHT)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = Rect::new(frame.x, frame.y, frame.width, SLIDER_HEIGHT);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let (start, length) = self.track_x();
        let track_y = (self.frame.y + (self.frame.height - TRACK_HEIGHT) / 2.0).round();
        items.push(Item::Fill {
            shape: Shape::rounded(
                Rect::new(start, track_y, length, TRACK_HEIGHT),
                TRACK_HEIGHT / 2.0,
            ),
            paint: cx.palette.well.into(),
        });
        if self.has_detents() {
            let interval = length / (self.ticks - 1) as f32;
            for tick in 0..self.ticks {
                items.push(Item::Fill {
                    shape: Shape::rect(Rect::new(
                        start + tick as f32 * interval - TICK_SIZE.0 / 2.0,
                        track_y + TRACK_HEIGHT + TICK_GAP,
                        TICK_SIZE.0,
                        TICK_SIZE.1,
                    )),
                    paint: cx.palette.ink3.into(),
                });
            }
        }
        let filled = length * cx.sample(&self.knob).clamp(0.0, 1.0);
        items.push(Item::Fill {
            shape: Shape::rounded(
                Rect::new(start, track_y, filled, TRACK_HEIGHT),
                TRACK_HEIGHT / 2.0,
            ),
            paint: cx.palette.accent.into(),
        });
        let knob = Shape::rounded(
            Rect::new(
                start + filled - KNOB_WIDTH / 2.0,
                self.frame.y,
                KNOB_WIDTH,
                self.frame.height,
            ),
            KNOB_RADIUS,
        );
        items.push(Item::Shadow {
            shape: knob,
            offset: Point::new(0.0, 1.0),
            blur_radius: 2.0,
            color: Color::BLACK.with_opacity(KNOB_SHADOW),
        });
        items.push(Item::Fill {
            shape: knob,
            paint: Color::WHITE.into(),
        });
        outline(knob, cx.palette.edge, items);
        if cx.is_focused(self.id) {
            super::focus_ring(cx, knob, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Slider);
        node.set_label(self.label.clone());
        node.set_numeric_value(self.value);
        node.set_min_numeric_value(self.range.0);
        node.set_max_numeric_value(self.range.1);
        node.set_numeric_value_step(self.interval());
        node.add_action(Action::Increment);
        node.add_action(Action::Decrement);
        node.add_action(Action::SetValue);
        node
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => {
                self.dragging = true;
                self.set(cx, self.value_at(pointer.position), self.has_detents());
            }
            Event::PointerMove(pointer) if self.dragging => {
                self.set(cx, self.value_at(pointer.position), self.has_detents());
            }
            Event::PointerUp(pointer) if pointer.is_primary() => self.dragging = false,
            Event::Key {
                key: Key::Right | Key::Up,
                ..
            } => self.set(cx, self.value + self.interval(), true),
            Event::Key {
                key: Key::Left | Key::Down,
                ..
            } => self.set(cx, self.value - self.interval(), true),
            _ => return false,
        }
        true
    }

    fn perform(&mut self, cx: &mut Context, request: &ActionRequest) -> bool {
        match (request.action, &request.data) {
            (Action::Increment, _) => self.set(cx, self.value + self.interval(), true),
            (Action::Decrement, _) => self.set(cx, self.value - self.interval(), true),
            (Action::SetValue, Some(ActionData::NumericValue(value))) => {
                self.set(cx, *value, true);
            }
            _ => return false,
        }
        true
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation
    }
}
