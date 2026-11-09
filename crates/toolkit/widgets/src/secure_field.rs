use std::ops::Range;

use accesskit::{Action, ActionData, ActionRequest, Node, Role};
use bande_l10n::tr;

use super::control::paint_field;
use super::{Context, Cursor, Event, Key, Modifiers, Size, Widget, WidgetId};
use bande_graphics::{
    ControlSize, Item, Paragraph, Path, Point, Radius, Rect, Shape, TextAlign, TextStyle,
};
use zeroize::Zeroizing;

const BULLET: char = '\u{25CF}';
const PADDING: f32 = 8.0;
const CARET_WIDTH: f32 = 1.5;
const CAPS_LOCK_SIZE: f32 = 12.0;
const CAPS_LOCK_STROKE: f32 = 1.25;
const BODY_FONT: f32 = 13.0;
const GO_INSET: f32 = 4.0;
const GO_STROKE: f32 = 1.5;

type OnEdit = Box<dyn FnMut(&str)>;

pub struct SecureField {
    id: WidgetId,
    placeholder: String,
    font_size: f32,
    scale: f32,
    secret: Zeroizing<String>,
    anchor: usize,
    caret: usize,
    on_submit: Box<dyn FnMut(&str)>,
    on_edit: Option<OnEdit>,
    bullets: Option<Paragraph>,
    bullet_width: f32,
    placeholder_paragraph: Option<Paragraph>,
    go_button: bool,
    frame: Rect,
}

impl SecureField {
    pub fn new(placeholder: &str, font_size: f32, on_submit: impl FnMut(&str) + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            placeholder: placeholder.to_owned(),
            font_size,
            scale: 1.0,
            secret: Zeroizing::default(),
            anchor: 0,
            caret: 0,
            on_submit: Box::new(on_submit),
            on_edit: None,
            bullets: None,
            bullet_width: 0.0,
            placeholder_paragraph: None,
            go_button: false,
            frame: Rect::default(),
        }
    }

    pub fn on_edit(mut self, on_edit: impl FnMut(&str) + 'static) -> Self {
        self.on_edit = Some(Box::new(on_edit));
        self
    }

    pub fn with_go_button(mut self) -> Self {
        self.go_button = true;
        self
    }

    fn go_rect(&self) -> Option<Rect> {
        (self.go_button && !self.secret.is_empty()).then(|| {
            let side = self.frame.height - 2.0 * GO_INSET;
            Rect::new(
                self.frame.x + self.frame.width - GO_INSET - side,
                self.frame.y + GO_INSET,
                side,
                side,
            )
        })
    }

    fn paint_go_button(&self, cx: &Context, items: &mut Vec<Item>) {
        let Some(rect) = self.go_rect() else {
            return;
        };
        items.push(Item::Fill {
            shape: Shape::rounded(rect, Radius::XS),
            paint: cx.palette.accent.into(),
        });
        let middle = Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
        let reach = rect.width / 5.0;
        let at = |dx: f32, dy: f32| Point::new(middle.x + dx * reach, middle.y + dy * reach);
        items.push(Item::StrokePath {
            path: Path::through(&[at(-1.0, 0.0), at(1.0, 0.0)]),
            width: GO_STROKE,
            paint: cx.palette.on_accent.into(),
        });
        items.push(Item::StrokePath {
            path: Path::through(&[at(0.2, -0.8), at(1.0, 0.0), at(0.2, 0.8)]),
            width: GO_STROKE,
            paint: cx.palette.on_accent.into(),
        });
    }

    pub fn set_placeholder(&mut self, placeholder: &str) {
        placeholder.clone_into(&mut self.placeholder);
    }

    pub fn secret(&self) -> &str {
        &self.secret
    }

    fn height(&self) -> f32 {
        (self.font_size * self.scale * 1.6 + PADDING)
            .ceil()
            .max(ControlSize::Regular.height())
    }

    fn paint_caps_lock(&self, cx: &Context, items: &mut Vec<Item>) {
        let size = CAPS_LOCK_SIZE * (self.font_size / BODY_FONT).max(1.0);
        let unit = size / CAPS_LOCK_SIZE;
        let right = self
            .go_rect()
            .map_or(self.frame.x + self.frame.width, |go| go.x);
        let x = right - PADDING - size;
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
            self.frame.x + PADDING - self.scroll(),
            self.frame.y + ((self.frame.height - height) / 2.0).round(),
        )
    }

    fn length(&self) -> usize {
        self.secret.chars().count()
    }

    fn advance(&self, index: usize) -> f32 {
        self.bullet_width * index as f32
    }

    fn scroll(&self) -> f32 {
        let right = self
            .go_rect()
            .map_or(self.frame.x + self.frame.width, |go| go.x);
        let room = right - self.frame.x - 2.0 * PADDING - CARET_WIDTH;
        (self.advance(self.caret) - room).max(0.0)
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }

    fn go(&mut self, to: usize, extend: bool) {
        self.caret = to;
        if !extend {
            self.anchor = to;
        }
    }

    fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.length();
    }

    fn replace(&mut self, range: Range<usize>, text: &str) {
        let byte = |index: usize| {
            self.secret
                .char_indices()
                .nth(index)
                .map_or(self.secret.len(), |(at, _)| at)
        };
        let (start, end) = (byte(range.start), byte(range.end));
        let mut replaced = Zeroizing::new(String::with_capacity(
            self.secret
                .capacity()
                .max(self.secret.len() - (end - start) + text.len()),
        ));
        replaced.push_str(&self.secret[..start]);
        replaced.push_str(text);
        replaced.push_str(&self.secret[end..]);
        self.secret = replaced;
        self.go(range.start + text.chars().count(), false);
    }

    fn insert(&mut self, text: &str) {
        let typed: Zeroizing<String> = Zeroizing::new(
            text.chars()
                .filter(|character| !character.is_control())
                .collect(),
        );
        if !typed.is_empty() {
            self.replace(self.selection(), &typed);
        }
    }

    fn delete(&mut self, forward: bool, to_the_edge: bool) -> bool {
        let selection = self.selection();
        let range = match (selection.is_empty(), forward, to_the_edge) {
            (false, _, _) => selection,
            (true, true, true) => self.caret..self.length(),
            (true, true, false) => self.caret..(self.caret + 1).min(self.length()),
            (true, false, true) => 0..self.caret,
            (true, false, false) => self.caret.saturating_sub(1)..self.caret,
        };
        let deletes = !range.is_empty();
        if deletes {
            self.replace(range, "");
        }
        deletes
    }

    fn arrow(&mut self, right: bool, modifiers: Modifiers) {
        let selection = self.selection();
        let to = match (right, modifiers.control) {
            (_, false) if !modifiers.shift && !selection.is_empty() => match right {
                true => selection.end,
                false => selection.start,
            },
            (true, true) => self.length(),
            (false, true) => 0,
            (true, false) => (self.caret + 1).min(self.length()),
            (false, false) => self.caret.saturating_sub(1),
        };
        self.go(to, modifiers.shift);
    }

    fn index_at(&self, x: f32) -> usize {
        if self.bullet_width <= 0.0 {
            return 0;
        }
        let origin = self.text_origin(self.bullets.as_ref()).x;
        (((x - origin) / self.bullet_width).round().max(0.0) as usize).min(self.length())
    }

    fn edited(&mut self) {
        if let Some(on_edit) = &mut self.on_edit {
            on_edit(&self.secret);
        }
    }

    fn key(&mut self, cx: &mut Context, key: Key, modifiers: Modifiers) -> Option<bool> {
        let typing = !modifiers.control && !modifiers.alt;
        match key {
            Key::Enter => (self.on_submit)(&self.secret),
            Key::Character('a') if modifiers.control => self.select_all(),
            Key::Character('c' | 'x') if modifiers.control => {}
            Key::Character('v') if modifiers.control => cx.request_paste(),
            Key::Left | Key::Right if !modifiers.alt => self.arrow(key == Key::Right, modifiers),
            Key::Home | Key::End if !modifiers.alt => {
                let to = if key == Key::End { self.length() } else { 0 };
                self.go(to, modifiers.shift);
            }
            Key::Backspace | Key::Delete => {
                return Some(self.delete(key == Key::Delete, modifiers.control));
            }
            Key::Character(character) if typing => {
                self.insert(character.encode_utf8(&mut [0; 4]));
                return Some(true);
            }
            Key::Space if typing => {
                self.insert(" ");
                return Some(true);
            }
            _ => return None,
        }
        Some(false)
    }
}

impl Widget for SecureField {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.scale = cx.text.scale();
        let count = self.length();
        let bullets = cx.text.paragraph(
            &std::iter::repeat_n(BULLET, count.max(1)).collect::<String>(),
            TextStyle::new(self.font_size, cx.palette.ink),
            None,
            TextAlign::Start,
        );
        self.bullet_width = bullets.width() / count.max(1) as f32;
        self.bullets = (count > 0).then_some(bullets);
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
        self.paint_go_button(cx, items);
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
        let height = shown.map_or(self.font_size, Paragraph::height);
        let selection = self.selection();
        if focused && !selection.is_empty() {
            let start = origin.x + self.advance(selection.start);
            content.insert(
                0,
                Item::Fill {
                    shape: Shape::rect(Rect::new(
                        start,
                        origin.y,
                        origin.x + self.advance(selection.end) - start,
                        height,
                    )),
                    paint: cx.palette.accent_soft.into(),
                },
            );
        } else if focused {
            content.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    origin.x + self.advance(self.caret),
                    origin.y,
                    CARET_WIDTH,
                    height,
                )),
                paint: cx.palette.accent_text.into(),
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
            node.set_description(tr!("Caps Lock is on"));
        }
        node.add_action(Action::Focus);
        node.add_action(Action::SetValue);
        node
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        match (&request.action, &request.data) {
            (Action::SetValue, Some(ActionData::Value(value))) => {
                self.secret = Zeroizing::new(value.to_string());
                self.go(self.length(), false);
                true
            }
            _ => false,
        }
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        let edited = match event {
            Event::Key { key, modifiers } if !modifiers.logo => {
                match self.key(cx, *key, *modifiers) {
                    Some(edited) => edited,
                    None => return false,
                }
            }
            Event::Commit(text) => {
                self.insert(text);
                true
            }
            Event::PointerDown(pointer) if pointer.is_primary() => {
                if self
                    .go_rect()
                    .is_some_and(|go| go.contains(pointer.position))
                {
                    (self.on_submit)(&self.secret);
                } else if pointer.count >= 2 {
                    self.select_all();
                } else {
                    let to = self.index_at(pointer.position.x);
                    self.go(to, pointer.modifiers.shift);
                }
                false
            }
            _ => return false,
        };
        if edited {
            self.edited();
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
