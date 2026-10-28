use accesskit::{Action, ActionRequest, Node, Role, Toggled};

use super::{Context, Event, Key, Size, Widget, WidgetId, contains};
use crate::scene::{Item, Rect, Shape};
use crate::tokens::Color;

pub const SWITCH_SIZE: Size = Size::new(38.0, 22.0);
const KNOB_INSET: f32 = 1.0;

pub struct Toggle {
    id: WidgetId,
    label: String,
    on: bool,
    on_change: Box<dyn FnMut(bool)>,
    pressed: bool,
    frame: Rect,
}

impl Toggle {
    pub fn new(label: &str, on: bool, on_change: impl FnMut(bool) + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            on,
            on_change: Box::new(on_change),
            pressed: false,
            frame: Rect::default(),
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    fn flip(&mut self) {
        self.on = !self.on;
        (self.on_change)(self.on);
    }
}

impl Widget for Toggle {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, _available: Size) -> Size {
        SWITCH_SIZE
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = Rect::new(frame.x, frame.y, SWITCH_SIZE.width, SWITCH_SIZE.height);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let track = Shape::rounded(self.frame, self.frame.height / 2.0);
        items.push(Item::Fill {
            shape: track,
            paint: if self.on {
                cx.palette.accent_color
            } else {
                cx.palette.separator_color
            }
            .into(),
        });
        let diameter = self.frame.height - 2.0 * KNOB_INSET;
        let x = if self.on {
            self.frame.x + self.frame.width - KNOB_INSET - diameter
        } else {
            self.frame.x + KNOB_INSET
        };
        items.push(Item::Fill {
            shape: Shape::rounded(
                Rect::new(x, self.frame.y + KNOB_INSET, diameter, diameter),
                diameter / 2.0,
            ),
            paint: Color::hex(0xFFFFFF).into(),
        });
        if cx.is_focused(self.id) {
            super::focus_ring(cx, track, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Switch);
        node.set_label(self.label.clone());
        node.set_toggled(if self.on {
            Toggled::True
        } else {
            Toggled::False
        });
        node.add_action(Action::Click);
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(_) => self.pressed = true,
            Event::PointerUp(point) => {
                let activate = self.pressed && contains(self.frame, *point);
                self.pressed = false;
                if activate {
                    self.flip();
                }
            }
            Event::Key {
                key: Key::Space, ..
            } => self.flip(),
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action == Action::Click {
            self.flip();
            return true;
        }
        false
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation
    }
}
