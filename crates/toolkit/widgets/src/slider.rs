use accesskit::{Action, ActionData, ActionRequest, Node, Role};

use super::{Context, Event, Key, Size, Widget, WidgetId};
use unbound_graphics::{Color, Item, Point, Rect, Shape};

pub const SLIDER_HEIGHT: f32 = 20.0;
const TRACK_HEIGHT: f32 = 4.0;

pub struct Slider {
    id: WidgetId,
    label: String,
    value: f64,
    range: (f64, f64),
    step: f64,
    on_change: Box<dyn FnMut(f64)>,
    dragging: bool,
    frame: Rect,
}

impl Slider {
    pub fn new(
        label: &str,
        value: f64,
        range: (f64, f64),
        step: f64,
        on_change: impl FnMut(f64) + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            value: value.clamp(range.0, range.1),
            range,
            step,
            on_change: Box::new(on_change),
            dragging: false,
            frame: Rect::default(),
        }
    }

    pub fn value(&self) -> f64 {
        self.value
    }

    fn set(&mut self, value: f64) {
        let value = value.clamp(self.range.0, self.range.1);
        if value != self.value {
            self.value = value;
            (self.on_change)(value);
        }
    }

    fn knob(&self) -> f32 {
        self.frame.height
    }

    fn fraction(&self) -> f64 {
        let span = self.range.1 - self.range.0;
        if span > 0.0 {
            (self.value - self.range.0) / span
        } else {
            0.0
        }
    }

    fn track_x(&self) -> (f32, f32) {
        let half = self.knob() / 2.0;
        (self.frame.x + half, self.frame.width - self.knob())
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
        let track_y = self.frame.y + (self.frame.height - TRACK_HEIGHT) / 2.0;
        let track = Rect::new(start, track_y, length, TRACK_HEIGHT);
        items.push(Item::Fill {
            shape: Shape::rounded(track, TRACK_HEIGHT / 2.0),
            paint: cx.palette.separator_color.into(),
        });
        let filled = length * self.fraction() as f32;
        items.push(Item::Fill {
            shape: Shape::rounded(
                Rect::new(start, track_y, filled, TRACK_HEIGHT),
                TRACK_HEIGHT / 2.0,
            ),
            paint: cx.palette.accent_color.into(),
        });
        let knob = Shape::rounded(
            Rect::new(
                start + filled - self.knob() / 2.0,
                self.frame.y,
                self.knob(),
                self.knob(),
            ),
            self.knob() / 2.0,
        );
        items.push(Item::Fill {
            shape: knob,
            paint: Color::hex(0xFFFFFF).into(),
        });
        items.push(Item::Stroke {
            shape: Shape::rounded(knob.rect.inset(0.5), knob.radius - 0.5),
            width: 1.0,
            paint: cx.palette.separator_color.into(),
        });
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
        node.set_numeric_value_step(self.step);
        node.add_action(Action::Increment);
        node.add_action(Action::Decrement);
        node.add_action(Action::SetValue);
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => {
                self.dragging = true;
                self.set(self.value_at(pointer.position));
            }
            Event::PointerMove(pointer) if self.dragging => {
                self.set(self.value_at(pointer.position));
            }
            Event::PointerUp(pointer) if pointer.is_primary() => self.dragging = false,
            Event::Key {
                key: Key::Right | Key::Up,
                ..
            } => self.set(self.value + self.step),
            Event::Key {
                key: Key::Left | Key::Down,
                ..
            } => self.set(self.value - self.step),
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        match (request.action, &request.data) {
            (Action::Increment, _) => self.set(self.value + self.step),
            (Action::Decrement, _) => self.set(self.value - self.step),
            (Action::SetValue, Some(ActionData::NumericValue(value))) => self.set(*value),
            _ => return false,
        }
        true
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation
    }
}
