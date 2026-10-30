use accesskit::{Action, ActionRequest, Node, Role, Toggled};

use super::{Animated, Context, Event, Key, Size, Spring, Widget, WidgetId, contains};
use unbound_graphics::{Color, Item, Rect, Shape};

pub const SWITCH_SIZE: Size = Size::new(38.0, 22.0);
const KNOB_INSET: f32 = 1.0;

pub struct Toggle {
    id: WidgetId,
    label: String,
    on: bool,
    on_change: Box<dyn FnMut(bool)>,
    pressed: bool,
    knob: Animated,
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
            knob: Animated::new(if on { 1.0 } else { 0.0 }),
            frame: Rect::default(),
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    fn flip(&mut self, cx: &Context) {
        self.on = !self.on;
        self.knob
            .animate_to(cx, if self.on { 1.0 } else { 0.0 }, Spring::QUICK);
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
        let progress = self.knob.value(cx);
        items.push(Item::Fill {
            shape: track,
            paint: cx.palette.hairline.mix(cx.palette.accent, progress).into(),
        });
        let diameter = self.frame.height - 2.0 * KNOB_INSET;
        let travel = self.frame.width - 2.0 * KNOB_INSET - diameter;
        let x = self.frame.x + KNOB_INSET + travel * progress;
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

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => self.pressed = true,
            Event::PointerUp(pointer) if pointer.is_primary() => {
                let activate = self.pressed && contains(self.frame, pointer.position);
                self.pressed = false;
                if activate {
                    self.flip(cx);
                }
            }
            Event::Key {
                key: Key::Space, ..
            } => self.flip(cx),
            _ => return false,
        }
        true
    }

    fn perform(&mut self, cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action == Action::Click {
            self.flip(cx);
            return true;
        }
        false
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation
    }
}
