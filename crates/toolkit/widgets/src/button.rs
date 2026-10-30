use accesskit::{Action, ActionRequest, Node, Role};

use super::{Context, Event, Key, Size, Widget, WidgetId, contains};
use unbound_graphics::{Color, Item, Paragraph, Point, Rect, Shape, TextAlign, TextStyle};

pub const BUTTON_HEIGHT: f32 = 22.0;
const RADIUS: f32 = 5.0;
const PADDING: f32 = 12.0;

pub struct Button {
    id: WidgetId,
    title: String,
    on_click: Box<dyn FnMut()>,
    pressed: bool,
    role: ButtonRole,
    tooltip: Option<String>,
    paragraph: Option<Paragraph>,
    frame: Rect,
}

impl Button {
    pub fn new(title: &str, on_click: impl FnMut() + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            title: title.to_owned(),
            on_click: Box::new(on_click),
            pressed: false,
            role: ButtonRole::Plain,
            tooltip: None,
            paragraph: None,
            frame: Rect::default(),
        }
    }

    pub fn primary(self) -> Self {
        Self {
            role: ButtonRole::Primary,
            ..self
        }
    }

    pub fn tooltip(self, tooltip: &str) -> Self {
        Self {
            tooltip: Some(tooltip.to_owned()),
            ..self
        }
    }

    pub fn cancel(self) -> Self {
        Self {
            role: ButtonRole::Cancel,
            ..self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonRole {
    Plain,
    Primary,
    Cancel,
}

impl Widget for Button {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        let style = match self.role {
            ButtonRole::Primary => {
                TextStyle::new(13.0, Color::hex(0xFFFFFF)).weight(TextStyle::MEDIUM)
            }
            _ => TextStyle::new(13.0, cx.palette.ink),
        };
        let paragraph = cx
            .text
            .paragraph(&self.title, style, None, TextAlign::Start);
        let size = Size::new((paragraph.width() + 2.0 * PADDING).ceil(), BUTTON_HEIGHT);
        self.paragraph = Some(paragraph);
        size
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let shape = Shape::rounded(self.frame, RADIUS);
        if self.role == ButtonRole::Primary {
            items.push(Item::Fill {
                shape,
                paint: cx.palette.accent.into(),
            });
            if self.pressed {
                items.push(Item::Fill {
                    shape,
                    paint: Color::hex(0x000000).with_alpha(48).into(),
                });
            }
        } else {
            let fill = if self.pressed {
                cx.palette.hairline
            } else {
                cx.palette.overlay
            };
            items.push(Item::Fill {
                shape,
                paint: fill.into(),
            });
            items.push(Item::Stroke {
                shape: Shape::rounded(self.frame.inset(0.5), RADIUS - 0.5),
                width: 1.0,
                paint: cx.palette.hairline.into(),
            });
        }
        if cx.is_focused(self.id) {
            super::focus_ring(cx, shape, items);
        }
        if let Some(paragraph) = &self.paragraph {
            items.push(Item::Text {
                origin: Point::new(
                    self.frame.x + ((self.frame.width - paragraph.width()) / 2.0),
                    self.frame.y + ((self.frame.height - paragraph.height()) / 2.0).round(),
                ),
                paragraph: paragraph.clone(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Button);
        node.set_label(self.title.clone());
        node.add_action(Action::Click);
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => self.pressed = true,
            Event::PointerMove(pointer) if self.pressed => {
                self.pressed = contains(self.frame, pointer.position);
            }
            Event::PointerUp(pointer) if pointer.is_primary() => {
                let activate = self.pressed && contains(self.frame, pointer.position);
                self.pressed = false;
                if activate {
                    (self.on_click)();
                }
            }
            Event::Key {
                key: Key::Space, ..
            } => (self.on_click)(),
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action == Action::Click {
            (self.on_click)();
            return true;
        }
        false
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation
    }

    fn tooltip(&self) -> Option<&str> {
        self.tooltip.as_deref()
    }

    fn key_equivalent(&self) -> Option<Key> {
        match self.role {
            ButtonRole::Plain => None,
            ButtonRole::Primary => Some(Key::Enter),
            ButtonRole::Cancel => Some(Key::Escape),
        }
    }
}
