use accesskit::{Action, ActionData, ActionRequest, Node, Role};
use parley::{PlainEditor, StyleProperty};

use super::{Context, Event, Key, Modifiers, Size, Widget, WidgetId};
use unbound_graphics::{
    Color, Item, Paint, Paragraph, Point, Rect, Shape, TextAlign, TextStyle, families,
};

pub const TEXT_FIELD_HEIGHT: f32 = 22.0;
const FONT_SIZE: f32 = 13.0;
const RADIUS: f32 = 5.0;
const PADDING: f32 = 6.0;
const CARET_WIDTH: f32 = 1.0;

pub struct TextField {
    id: WidgetId,
    placeholder: String,
    placeholder_paragraph: Option<Paragraph>,
    font_size: f32,
    bordered: bool,
    editor: PlainEditor<Color>,
    on_change: Box<dyn FnMut(&str)>,
    selecting: bool,
    paragraph: Option<Paragraph>,
    frame: Rect,
}

impl TextField {
    pub fn new(placeholder: &str, text: &str, on_change: impl FnMut(&str) + 'static) -> Self {
        let mut editor = PlainEditor::new(FONT_SIZE);
        editor.set_text(text);
        editor
            .edit_styles()
            .insert(StyleProperty::FontFamily(families(FONT_SIZE)));
        Self {
            id: WidgetId::next(),
            placeholder: placeholder.to_owned(),
            placeholder_paragraph: None,
            font_size: FONT_SIZE,
            bordered: true,
            editor,
            on_change: Box::new(on_change),
            selecting: false,
            paragraph: None,
            frame: Rect::default(),
        }
    }

    pub fn borderless(mut self, font_size: f32) -> Self {
        let text = self.editor.raw_text().to_owned();
        self.editor = PlainEditor::new(font_size);
        self.editor.set_text(&text);
        self.editor
            .edit_styles()
            .insert(StyleProperty::FontFamily(families(font_size)));
        self.font_size = font_size;
        self.bordered = false;
        self
    }

    fn height(&self) -> f32 {
        if self.bordered {
            TEXT_FIELD_HEIGHT
        } else {
            (self.font_size * 1.5).ceil()
        }
    }

    pub fn text(&self) -> &str {
        self.editor.raw_text()
    }

    pub fn selection(&self) -> std::ops::Range<usize> {
        self.editor.raw_selection().text_range()
    }

    fn text_origin(&self) -> Point {
        let height = self.paragraph.as_ref().map_or(0.0, Paragraph::height);
        Point::new(
            self.frame.x + PADDING,
            self.frame.y + ((self.frame.height - height) / 2.0).round(),
        )
    }

    fn edit(&mut self, cx: &mut Context, key: Key, modifiers: Modifiers) -> bool {
        let before = self.editor.raw_text().to_owned();
        let (fonts, layouts) = cx.text.contexts();
        let mut driver = self.editor.driver(fonts, layouts);
        let Modifiers {
            shift, alt, logo, ..
        } = modifiers;
        match key {
            Key::Left if logo && shift => driver.select_to_line_start(),
            Key::Left if logo => driver.move_to_line_start(),
            Key::Left if alt && shift => driver.select_word_left(),
            Key::Left if alt => driver.move_word_left(),
            Key::Left if shift => driver.select_left(),
            Key::Left => driver.move_left(),
            Key::Right if logo && shift => driver.select_to_line_end(),
            Key::Right if logo => driver.move_to_line_end(),
            Key::Right if alt && shift => driver.select_word_right(),
            Key::Right if alt => driver.move_word_right(),
            Key::Right if shift => driver.select_right(),
            Key::Right => driver.move_right(),
            Key::Up if shift => driver.select_to_text_start(),
            Key::Up => driver.move_to_text_start(),
            Key::Down if shift => driver.select_to_text_end(),
            Key::Down => driver.move_to_text_end(),
            Key::Backspace if logo => {
                if driver.editor.raw_selection().is_collapsed() {
                    driver.select_to_line_start();
                }
                driver.delete_selection();
            }
            Key::Backspace if alt => driver.backdelete_word(),
            Key::Backspace => driver.backdelete(),
            Key::Delete if alt => driver.delete_word(),
            Key::Delete => driver.delete(),
            Key::Character('a') if logo => driver.select_all(),
            Key::Character(character) if !logo && !modifiers.control => {
                driver.insert_or_replace_selection(character.encode_utf8(&mut [0; 4]));
            }
            Key::Space if !logo && !modifiers.control => driver.insert_or_replace_selection(" "),
            _ => return false,
        }
        if self.editor.raw_text() != before {
            (self.on_change)(self.editor.raw_text());
        }
        true
    }
}

impl Widget for TextField {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.editor
            .edit_styles()
            .insert(StyleProperty::Brush(cx.palette.label_primary));
        let (fonts, layouts) = cx.text.contexts();
        self.paragraph = Some(Paragraph::from_layout(
            self.editor.layout(fonts, layouts).clone(),
        ));
        self.placeholder_paragraph = (!self.placeholder.is_empty()).then(|| {
            cx.text.paragraph(
                &self.placeholder,
                TextStyle::new(self.font_size, cx.palette.label_muted),
                None,
                TextAlign::Start,
            )
        });
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
        let focused = cx.is_focused(self.id);
        if self.bordered {
            items.push(Item::Fill {
                shape,
                paint: cx.palette.surface_elevated.into(),
            });
            items.push(Item::Stroke {
                shape: Shape::rounded(self.frame.inset(0.5), RADIUS - 0.5),
                width: 1.0,
                paint: cx.palette.separator_color.into(),
            });
            if focused {
                super::focus_ring(cx, shape, items);
            }
        }
        let origin = self.text_origin();
        let mut content = Vec::new();
        if self.editor.raw_text().is_empty()
            && let Some(placeholder) = &self.placeholder_paragraph
        {
            content.push(Item::Text {
                origin,
                paragraph: placeholder.clone(),
            });
        }
        self.editor.selection_geometry_with(|bounds, _| {
            content.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    origin.x + bounds.x0 as f32,
                    origin.y + bounds.y0 as f32,
                    (bounds.x1 - bounds.x0) as f32,
                    (bounds.y1 - bounds.y0) as f32,
                )),
                paint: Paint::Solid(cx.palette.accent_color.with_alpha(80)),
            });
        });
        if let Some(paragraph) = &self.paragraph {
            content.push(Item::Text {
                origin,
                paragraph: paragraph.clone(),
            });
        }
        if focused
            && self.editor.raw_selection().is_collapsed()
            && let Some(caret) = self.editor.cursor_geometry(CARET_WIDTH)
        {
            content.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    origin.x + caret.x0 as f32,
                    origin.y + caret.y0 as f32,
                    CARET_WIDTH,
                    (caret.y1 - caret.y0) as f32,
                )),
                paint: cx.palette.label_primary.into(),
            });
        }
        items.push(Item::Clip {
            shape: Shape::rect(self.frame.inset(1.0)),
            items: content,
        });
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::TextInput);
        node.set_value(self.editor.raw_text().to_owned());
        node.set_placeholder(self.placeholder.clone());
        node.add_action(Action::SetValue);
        node.add_action(Action::ReplaceSelectedText);
        node
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        let origin = self.text_origin();
        let (fonts, layouts) = cx.text.contexts();
        match event {
            Event::PointerDown(point) => {
                self.selecting = true;
                self.editor
                    .driver(fonts, layouts)
                    .move_to_point(point.x - origin.x, point.y - origin.y);
            }
            Event::PointerMove(point) if self.selecting => {
                self.editor
                    .driver(fonts, layouts)
                    .extend_selection_to_point(point.x - origin.x, point.y - origin.y);
            }
            Event::PointerUp(_) => self.selecting = false,
            Event::Key { key, modifiers } => return self.edit(cx, *key, *modifiers),
            Event::Commit(text) => {
                let mut driver = self.editor.driver(fonts, layouts);
                driver.clear_compose();
                driver.insert_or_replace_selection(text);
                (self.on_change)(self.editor.raw_text());
            }
            Event::Preedit { text, .. } if text.is_empty() => {
                if !self.editor.is_composing() {
                    return false;
                }
                self.editor.driver(fonts, layouts).clear_compose();
            }
            Event::Preedit { text, cursor } => {
                self.editor
                    .driver(fonts, layouts)
                    .set_compose(text, *cursor);
            }
            Event::PointerMove(_) => return false,
        }
        true
    }

    fn perform(&mut self, cx: &mut Context, request: &ActionRequest) -> bool {
        let Some(ActionData::Value(value)) = &request.data else {
            return false;
        };
        let (fonts, layouts) = cx.text.contexts();
        match request.action {
            Action::SetValue => {
                self.editor.set_text(value);
                self.editor.driver(fonts, layouts).move_to_text_end();
            }
            Action::ReplaceSelectedText => self
                .editor
                .driver(fonts, layouts)
                .insert_or_replace_selection(value),
            _ => return false,
        }
        (self.on_change)(self.editor.raw_text());
        true
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }

    fn text_input_area(&self) -> Option<Rect> {
        let origin = self.text_origin();
        let area = self.editor.ime_cursor_area();
        Some(Rect::new(
            origin.x + area.x0 as f32,
            origin.y + area.y0 as f32,
            (area.x1 - area.x0) as f32,
            (area.y1 - area.y0) as f32,
        ))
    }
}
