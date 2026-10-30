use accesskit::{Action, ActionRequest, Node, Role, Toggled};
use unbound_graphics::{Color, Item, Paragraph, Path, Point, Rect, Shape, TextAlign, TextStyle};

use super::{Animated, Context, Event, Key, Size, Spring, Widget, WidgetId, contains};

pub const CHECKBOX_SIZE: f32 = 14.0;
const RADIUS: f32 = 3.5;
const GAP: f32 = 6.0;
const FONT_SIZE: f32 = 13.0;
const MARK: f32 = 1.75;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CheckState {
    #[default]
    Off,
    On,
    Mixed,
}

pub struct Checkbox {
    id: WidgetId,
    label: String,
    state: CheckState,
    on_change: Box<dyn FnMut(bool)>,
    pressed: bool,
    shown: Animated,
    paragraph: Option<Paragraph>,
    width: f32,
    frame: Rect,
}

fn level(state: CheckState) -> f32 {
    match state {
        CheckState::Off => 0.0,
        CheckState::On | CheckState::Mixed => 1.0,
    }
}

impl Checkbox {
    pub fn new(label: &str, state: CheckState, on_change: impl FnMut(bool) + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            state,
            on_change: Box::new(on_change),
            pressed: false,
            shown: Animated::new(level(state)),
            paragraph: None,
            width: 0.0,
            frame: Rect::default(),
        }
    }

    pub fn state(&self) -> CheckState {
        self.state
    }

    pub fn set_state(&mut self, cx: &Context, state: CheckState) {
        self.state = state;
        self.shown.animate_to(cx, level(state), Spring::QUICK);
    }

    fn toggle(&mut self, cx: &Context) {
        let on = self.state != CheckState::On;
        self.set_state(
            cx,
            match on {
                true => CheckState::On,
                false => CheckState::Off,
            },
        );
        (self.on_change)(on);
    }

    fn box_rect(&self) -> Rect {
        Rect::new(
            self.frame.x,
            self.frame.y + ((self.frame.height - CHECKBOX_SIZE) / 2.0).round(),
            CHECKBOX_SIZE,
            CHECKBOX_SIZE,
        )
    }
}

pub(crate) fn paint_box(
    cx: &Context,
    rect: Rect,
    radius: f32,
    shown: f32,
    pressed: bool,
    items: &mut Vec<Item>,
) {
    let shape = Shape::rounded(rect, radius);
    items.push(Item::Fill {
        shape,
        paint: cx.palette.overlay.into(),
    });
    items.push(Item::Stroke {
        shape: Shape::rounded(rect.inset(0.5), radius - 0.5),
        width: 1.0,
        paint: cx.palette.hairline.mix(cx.palette.ink2, 0.35).into(),
    });
    if shown > 0.0 {
        items.push(Item::Fill {
            shape,
            paint: cx.palette.accent.with_alpha((shown * 255.0) as u8).into(),
        });
    }
    if pressed {
        items.push(Item::Fill {
            shape,
            paint: Color::hex(0).with_alpha(36).into(),
        });
    }
}

impl Widget for Checkbox {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        let paragraph = cx.text.paragraph(
            &self.label,
            TextStyle::new(FONT_SIZE, cx.palette.ink),
            None,
            TextAlign::Start,
        );
        let size = Size::new(
            (CHECKBOX_SIZE + GAP + paragraph.width()).ceil(),
            paragraph.height().max(CHECKBOX_SIZE).ceil(),
        );
        self.paragraph = Some(paragraph);
        self.width = size.width;
        size
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = Rect::new(frame.x, frame.y, self.width.min(frame.width), frame.height);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let rect = self.box_rect();
        let shown = self.shown.value(cx);
        paint_box(cx, rect, RADIUS, shown, self.pressed, items);
        if cx.is_focused(self.id) {
            super::focus_ring(cx, Shape::rounded(rect, RADIUS), items);
        }
        let mark = Color::hex(0xFFFFFF).with_alpha((shown * 255.0) as u8);
        let at = |x: f32, y: f32| Point::new(rect.x + x, rect.y + y);
        let path = match self.state {
            CheckState::Mixed => Path::new().move_to(at(3.5, 7.0)).line_to(at(10.5, 7.0)),
            _ => Path::new()
                .move_to(at(3.5, 7.25))
                .line_to(at(6.0, 9.75))
                .line_to(at(10.75, 4.25)),
        };
        if self.state != CheckState::Off || shown > 0.0 {
            items.push(Item::StrokePath {
                path,
                width: MARK,
                paint: mark.into(),
            });
        }
        if let Some(paragraph) = &self.paragraph {
            items.push(Item::Text {
                origin: Point::new(
                    rect.x + CHECKBOX_SIZE + GAP,
                    self.frame.y + ((self.frame.height - paragraph.height()) / 2.0).round(),
                ),
                paragraph: paragraph.clone(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::CheckBox);
        node.set_label(self.label.clone());
        node.set_toggled(match self.state {
            CheckState::Off => Toggled::False,
            CheckState::On => Toggled::True,
            CheckState::Mixed => Toggled::Mixed,
        });
        node.add_action(Action::Click);
        node
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => self.pressed = true,
            Event::PointerMove(pointer) if self.pressed => {
                self.pressed = contains(self.frame, pointer.position);
            }
            Event::PointerUp(pointer) if pointer.is_primary() => {
                let activate = self.pressed && contains(self.frame, pointer.position);
                self.pressed = false;
                if activate {
                    self.toggle(cx);
                }
            }
            Event::Key {
                key: Key::Space, ..
            } => self.toggle(cx),
            _ => return false,
        }
        true
    }

    fn perform(&mut self, cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action == Action::Click {
            self.toggle(cx);
            return true;
        }
        false
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation
    }
}
