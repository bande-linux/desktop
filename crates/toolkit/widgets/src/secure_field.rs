use accesskit::{Action, ActionData, ActionRequest, Node, Role};

use super::control::paint_field;
use super::{Context, Cursor, Event, Key, Size, Widget, WidgetId};
use crate::L10N;
use unbound_graphics::{
    ControlSize, Item, Paragraph, Path, Point, Rect, Shape, TextAlign, TextStyle,
};

const BULLET: char = '\u{25CF}';
const PADDING: f32 = 8.0;
const CARET_WIDTH: f32 = 1.5;
const CAPS_LOCK_SIZE: f32 = 12.0;
const CAPS_LOCK_STROKE: f32 = 1.25;
const BODY_FONT: f32 = 13.0;

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

    pub fn set_placeholder(&mut self, placeholder: &str) {
        placeholder.clone_into(&mut self.placeholder);
    }

    fn height(&self) -> f32 {
        (self.font_size * 1.6 + PADDING)
            .ceil()
            .max(ControlSize::Regular.height())
    }

    fn paint_caps_lock(&self, cx: &Context, items: &mut Vec<Item>) {
        let size = CAPS_LOCK_SIZE * (self.font_size / BODY_FONT).max(1.0);
        let unit = size / CAPS_LOCK_SIZE;
        let x = self.frame.x + self.frame.width - PADDING - size;
        let y = self.frame.y + ((self.frame.height - size) / 2.0).round();
        let at = |dx: f32, dy: f32| Point::new(x + dx * unit, y + dy * unit);
        items.push(Item::StrokePath {
            path: Path::through(&[
                at(6.0, 0.5),
                at(11.5, 6.0),
                at(8.5, 6.0),
                at(8.5, 8.5),
                at(3.5, 8.5),
                at(3.5, 6.0),
                at(0.5, 6.0),
            ])
            .close(),
            width: CAPS_LOCK_STROKE * unit,
            paint: cx.palette.ink2.into(),
        });
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(
                at(3.5, 10.25).x - CAPS_LOCK_STROKE * unit / 2.0,
                at(3.5, 10.25).y,
                5.0 * unit + CAPS_LOCK_STROKE * unit,
                1.5 * unit,
            )),
            paint: cx.palette.ink2.into(),
        });
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
            TextStyle::new(self.font_size, cx.palette.ink),
            None,
            TextAlign::Start,
        ));
        self.placeholder_paragraph = Some(cx.text.paragraph(
            &self.placeholder,
            TextStyle::new(self.font_size, cx.palette.ink3),
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
        let focused = cx.is_focused(self.id);
        paint_field(cx, self.frame, focused, items);
        if focused && cx.caps_lock {
            self.paint_caps_lock(cx, items);
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
                paint: cx.palette.accent.into(),
            });
        }
        items.push(Item::Clip {
            shape: Shape::rect(self.frame.inset(1.0)),
            items: content,
        });
    }

    fn access(&self, cx: &Context) -> Node {
        let mut node = Node::new(Role::PasswordInput);
        node.set_placeholder(self.placeholder.clone());
        if cx.caps_lock && cx.is_focused(self.id) {
            node.set_description(L10N.tr("caps-lock-on"));
        }
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
            return matches!(event, Event::PointerDown(pointer) if pointer.is_primary());
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

    fn cursor(&self, _point: Point) -> Option<Cursor> {
        Some(Cursor::Text)
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }
}
