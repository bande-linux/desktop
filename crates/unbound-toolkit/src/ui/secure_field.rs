use accesskit::{Action, ActionData, ActionRequest, Node, Role};

use super::{Context, Event, Key, Size, Widget, WidgetId};
use crate::scene::{Item, Point, Rect, Shape};
use crate::text::{Paragraph, TextAlign, TextStyle};

const BULLET: char = '\u{25CF}';
const RADIUS: f32 = 6.0;
const PADDING: f32 = 8.0;
const CARET_WIDTH: f32 = 1.0;

pub struct SecureField {
    id: WidgetId,
    placeholder: String,
    font_size: f32,
    secret: String,
    on_submit: Box<dyn FnMut(&str)>,
    bullets: Option<Paragraph>,
    placeholder_paragraph: Option<Paragraph>,
    frame: Rect,
}

impl SecureField {
    pub fn new(placeholder: &str, font_size: f32, on_submit: impl FnMut(&str) + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            placeholder: placeholder.to_owned(),
            font_size,
            secret: String::new(),
            on_submit: Box::new(on_submit),
            bullets: None,
            placeholder_paragraph: None,
            frame: Rect::default(),
        }
    }

    fn height(&self) -> f32 {
        (self.font_size * 1.6 + PADDING).ceil()
    }

    fn text_origin(&self, paragraph: Option<&Paragraph>) -> Point {
        let height = paragraph.map_or(0.0, Paragraph::height);
        Point::new(
            self.frame.x + PADDING,
            self.frame.y + ((self.frame.height - height) / 2.0).round(),
        )
    }
}

impl Widget for SecureField {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let bullets: String = std::iter::repeat_n(BULLET, self.secret.chars().count()).collect();
        self.bullets = Some(cx.text.paragraph(
            &bullets,
            TextStyle::new(self.font_size, cx.palette.label_primary),
            None,
            TextAlign::Start,
        ));
        self.placeholder_paragraph = Some(cx.text.paragraph(
            &self.placeholder,
            TextStyle::new(self.font_size, cx.palette.label_muted),
            None,
            TextAlign::Start,
        ));
        Size::new(available.width, self.height())
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = Rect::new(frame.x, frame.y, frame.width, self.height());
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let shape = Shape::rounded(self.frame, RADIUS);
        items.push(Item::Fill {
            shape,
            paint: cx.palette.surface_elevated.into(),
        });
        items.push(Item::Stroke {
            shape: Shape::rounded(self.frame.inset(0.5), RADIUS - 0.5),
            width: 1.0,
            paint: cx.palette.separator_color.into(),
        });
        let focused = cx.is_focused(self.id);
        if focused {
            super::focus_ring(cx, shape, items);
        }
        let shown = if self.secret.is_empty() {
            self.placeholder_paragraph.as_ref()
        } else {
            self.bullets.as_ref()
        };
        let origin = self.text_origin(shown);
        let mut content = Vec::new();
        if let Some(paragraph) = shown {
            content.push(Item::Text {
                origin,
                paragraph: paragraph.clone(),
            });
        }
        if focused {
            let advance = if self.secret.is_empty() {
                0.0
            } else {
                self.bullets.as_ref().map_or(0.0, Paragraph::width)
            };
            let height = shown.map_or(self.font_size, Paragraph::height);
            content.push(Item::Fill {
                shape: Shape::rect(Rect::new(origin.x + advance, origin.y, CARET_WIDTH, height)),
                paint: cx.palette.label_primary.into(),
            });
        }
        items.push(Item::Clip {
            shape: Shape::rect(self.frame.inset(1.0)),
            items: content,
        });
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::PasswordInput);
        node.set_placeholder(self.placeholder.clone());
        node.add_action(Action::Focus);
        node.add_action(Action::SetValue);
        node
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        match (&request.action, &request.data) {
            (Action::SetValue, Some(ActionData::Value(value))) => {
                self.secret = value.to_string();
                true
            }
            _ => false,
        }
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        let Event::Key { key, modifiers } = event else {
            return matches!(event, Event::PointerDown(_));
        };
        if modifiers.control {
            return false;
        }
        match key {
            Key::Enter => (self.on_submit)(&self.secret),
            Key::Backspace if modifiers.logo || modifiers.alt => self.secret.clear(),
            Key::Backspace => {
                self.secret.pop();
            }
            Key::Character(character) if !modifiers.logo => self.secret.push(*character),
            Key::Space if !modifiers.logo => self.secret.push(' '),
            _ => return false,
        }
        true
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }
}
