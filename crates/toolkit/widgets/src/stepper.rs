use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Node, Role};
use unbound_graphics::{Color, Item, Path, Point, Rect, Shape};

use super::{Context, Event, Key, Size, Widget, WidgetId, contains};

pub const STEPPER_SIZE: Size = Size::new(15.0, 22.0);
const RADIUS: f32 = 5.0;
const CHEVRON_WIDTH: f32 = 3.0;
const CHEVRON_HEIGHT: f32 = 2.0;
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
    press: Option<Press>,
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
            press: None,
            frame: Rect::default(),
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
        if !contains(self.frame, point) {
            return None;
        }
        match point.y < self.frame.y + self.frame.height / 2.0 {
            true => Some(Half::Up),
            false => Some(Half::Down),
        }
    }

    fn half_rect(&self, half: Half) -> Rect {
        let height = self.frame.height / 2.0;
        let y = match half {
            Half::Up => self.frame.y,
            Half::Down => self.frame.y + height,
        };
        Rect::new(self.frame.x, y, self.frame.width, height)
    }
}

impl Widget for Stepper {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, _available: Size) -> Size {
        STEPPER_SIZE
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = Rect::new(frame.x, frame.y, STEPPER_SIZE.width, STEPPER_SIZE.height);
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
        let shape = Shape::rounded(self.frame, RADIUS);
        items.push(Item::Fill {
            shape,
            paint: cx.palette.overlay.into(),
        });
        if let Some(press) = self.press.as_ref().filter(|press| press.inside) {
            items.push(Item::Clip {
                shape,
                items: vec![Item::Fill {
                    shape: Shape::rect(self.half_rect(press.half)),
                    paint: Color::hex(0).with_alpha(30).into(),
                }],
            });
        }
        items.push(Item::Stroke {
            shape: Shape::rounded(self.frame.inset(0.5), RADIUS - 0.5),
            width: 1.0,
            paint: cx.palette.hairline.into(),
        });
        let middle = self.frame.y + (self.frame.height / 2.0).round();
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(
                self.frame.x + 3.0,
                middle - 0.5,
                self.frame.width - 6.0,
                1.0,
            )),
            paint: cx.palette.hairline.into(),
        });
        let x = self.frame.x + self.frame.width / 2.0;
        for (half, limit) in [(Half::Up, self.range.1), (Half::Down, self.range.0)] {
            let rect = self.half_rect(half);
            let centre = rect.y + rect.height / 2.0;
            let (tip, base) = match half {
                Half::Up => (centre - CHEVRON_HEIGHT / 2.0, centre + CHEVRON_HEIGHT / 2.0),
                Half::Down => (centre + CHEVRON_HEIGHT / 2.0, centre - CHEVRON_HEIGHT / 2.0),
            };
            let color = match self.value == limit {
                true => cx.palette.ink2.with_alpha(110),
                false => cx.palette.ink,
            };
            items.push(Item::StrokePath {
                path: Path::new()
                    .move_to(Point::new(x - CHEVRON_WIDTH, base))
                    .line_to(Point::new(x, tip))
                    .line_to(Point::new(x + CHEVRON_WIDTH, base)),
                width: 1.5,
                paint: color.into(),
            });
        }
        if cx.is_focused(self.id) {
            super::focus_ring(cx, shape, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::SpinButton);
        node.set_label(self.label.clone());
        node.set_numeric_value(self.value);
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
            Event::Key { key: Key::Up, .. } => {
                self.nudge(Half::Up);
            }
            Event::Key { key: Key::Down, .. } => {
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
