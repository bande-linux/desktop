use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Node, Role};
use bande_graphics::{
    ControlSize, Item, Paragraph, Point, Radius, Rect, Shape, TextAlign, TextStyle,
};

use super::control::{focus_ring, glyph_line};
use super::{Context, Event, Key, Size, Widget, WidgetId};

pub const STEPPER_SIZE: Size = Size::new(96.0, ControlSize::Regular.height());
const CELL: f32 = 28.0;
const ARM: f32 = 4.5;
const FONT_SIZE: f32 = 13.0;
const PRESSED_INK: f32 = 0.12;
const REPEAT_DELAY: Duration = Duration::from_millis(400);
const REPEAT_INTERVAL: Duration = Duration::from_millis(70);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Half {
    Up,
    Down,
}

struct Press {
    half: Half,
    inside: bool,
    next: Instant,
}

pub struct Stepper {
    id: WidgetId,
    label: String,
    value: f64,
    range: (f64, f64),
    step: f64,
    on_change: Box<dyn FnMut(f64)>,
    format: Box<dyn Fn(f64) -> String>,
    press: Option<Press>,
    paragraph: Option<Paragraph>,
    frame: Rect,
}

impl Stepper {
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
            format: Box::new(|value| value.to_string()),
            press: None,
            paragraph: None,
            frame: Rect::default(),
        }
    }

    pub fn format(self, format: impl Fn(f64) -> String + 'static) -> Self {
        Self {
            format: Box::new(format),
            ..self
        }
    }

    pub fn value(&self) -> f64 {
        self.value
    }

    pub fn set_value(&mut self, value: f64) {
        self.value = value.clamp(self.range.0, self.range.1);
    }

    fn nudge(&mut self, half: Half) -> bool {
        let delta = match half {
            Half::Up => self.step,
            Half::Down => -self.step,
        };
        let value = (self.value + delta).clamp(self.range.0, self.range.1);
        if value == self.value {
            return false;
        }
        self.value = value;
        (self.on_change)(value);
        true
    }

    fn half_at(&self, point: Point) -> Option<Half> {
        [Half::Down, Half::Up]
            .into_iter()
            .find(|half| self.cell(*half).contains(point))
    }

    fn cell(&self, half: Half) -> Rect {
        let x = match half {
            Half::Down => self.frame.x,
            Half::Up => self.frame.x + self.frame.width - CELL,
        };
        Rect::new(x, self.frame.y, CELL, self.frame.height)
    }
}

fn stepper_size(cx: &Context) -> Size {
    Size::new(
        (STEPPER_SIZE.width * cx.text.scale()).round(),
        cx.control_height(ControlSize::Regular),
    )
}

impl Widget for Stepper {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        self.paragraph = Some(
            cx.text.paragraph(
                &(self.format)(self.value),
                TextStyle::new(FONT_SIZE, cx.palette.ink)
                    .weight(TextStyle::MEDIUM)
                    .tabular(),
                None,
                TextAlign::Start,
            ),
        );
        stepper_size(cx)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        let size = stepper_size(cx);
        self.frame = Rect::new(frame.x, frame.y, size.width, size.height);
        let now = cx.now();
        let Some(press) = &mut self.press else {
            return;
        };
        cx.animating.set(true);
        if !press.inside || now < press.next {
            return;
        }
        press.next = now + REPEAT_INTERVAL;
        let half = press.half;
        self.nudge(half);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let shape = Shape::rounded(self.frame, Radius::S);
        items.push(Item::Fill {
            shape,
            paint: cx.palette.well.into(),
        });
        if let Some(press) = self.press.as_ref().filter(|press| press.inside) {
            items.push(Item::Clip {
                shape,
                items: vec![Item::Fill {
                    shape: Shape::rect(self.cell(press.half)),
                    paint: cx.palette.ink.with_opacity(PRESSED_INK).into(),
                }],
            });
        }
        for (half, limit) in [(Half::Up, self.range.1), (Half::Down, self.range.0)] {
            let cell = self.cell(half);
            let centre = Point::new(cell.x + cell.width / 2.0, cell.y + cell.height / 2.0);
            let color = match self.value == limit {
                true => cx.palette.ink3,
                false => cx.palette.ink2,
            };
            glyph_line(
                Point::new(centre.x - ARM, centre.y),
                Point::new(centre.x + ARM, centre.y),
                color,
                items,
            );
            if half == Half::Up {
                glyph_line(
                    Point::new(centre.x, centre.y - ARM),
                    Point::new(centre.x, centre.y + ARM),
                    color,
                    items,
                );
            }
        }
        if let Some(paragraph) = &self.paragraph {
            items.push(Item::Text {
                origin: Point::new(
                    self.frame.x + ((self.frame.width - paragraph.width()) / 2.0).round(),
                    self.frame.y + ((self.frame.height - paragraph.height()) / 2.0).round(),
                ),
                paragraph: paragraph.clone(),
            });
        }
        if cx.is_focused(self.id) {
            focus_ring(cx, shape, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::SpinButton);
        node.set_label(self.label.clone());
        node.set_numeric_value(self.value);
        node.set_value((self.format)(self.value));
        node.set_min_numeric_value(self.range.0);
        node.set_max_numeric_value(self.range.1);
        node.set_numeric_value_step(self.step);
        node.add_action(Action::Increment);
        node.add_action(Action::Decrement);
        node
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => {
                let Some(half) = self.half_at(pointer.position) else {
                    return false;
                };
                self.nudge(half);
                self.press = Some(Press {
                    half,
                    inside: true,
                    next: cx.now() + REPEAT_DELAY,
                });
                cx.animating.set(true);
            }
            Event::PointerMove(pointer) if self.press.is_some() => {
                let over = self.half_at(pointer.position);
                if let Some(press) = &mut self.press {
                    press.inside = over == Some(press.half);
                }
            }
            Event::PointerUp(pointer) if pointer.is_primary() => self.press = None,
            Event::PointerLeave if self.press.is_some() => self.press = None,
            Event::Key {
                key: Key::Up | Key::Right,
                ..
            } => {
                self.nudge(Half::Up);
            }
            Event::Key {
                key: Key::Down | Key::Left,
                ..
            } => {
                self.nudge(Half::Down);
            }
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        match request.action {
            Action::Increment => self.nudge(Half::Up),
            Action::Decrement => self.nudge(Half::Down),
            _ => false,
        }
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation
    }
}
