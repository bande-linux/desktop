use accesskit::{ActionRequest, Node, Role};

use std::ops::Range;

use super::editor::Editor;
use super::{Context, Cursor, Event, Menu, Parts, Size, Widget, WidgetId};
use unbound_graphics::{Color, Item, Paragraph, Point, Rect, Shape, TextAlign, TextStyle};

pub const TEXT_FIELD_HEIGHT: f32 = 22.0;
const FONT_SIZE: f32 = 13.0;
const RADIUS: f32 = 5.0;
const PADDING: f32 = 6.0;
const CARET_ROOM: f32 = 1.0;
const IN_PLACE_RADIUS: f32 = 4.0;
const SEARCH_RADIUS: f32 = 7.0;
const WHOLE_LINE: Range<f32> = 0.0..f32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Border {
    Field,
    InPlace,
    Search,
    None,
}

pub struct TextField {
    id: WidgetId,
    placeholder: String,
    placeholder_paragraph: Option<Paragraph>,
    border: Border,
    leading: f32,
    trailing: f32,
    editor: Editor,
    on_change: Box<dyn FnMut(&str)>,
    scroll: f32,
    frame: Rect,
}

impl TextField {
    pub fn new(placeholder: &str, text: &str, on_change: impl FnMut(&str) + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            placeholder: placeholder.to_owned(),
            placeholder_paragraph: None,
            border: Border::Field,
            leading: PADDING,
            trailing: PADDING,
            editor: Editor::new(text, FONT_SIZE, false),
            on_change: Box::new(on_change),
            scroll: 0.0,
            frame: Rect::default(),
        }
    }

    pub fn borderless(mut self, font_size: f32) -> Self {
        self.editor = Editor::new(&self.editor.text(), font_size, false);
        self.border = Border::None;
        self
    }

    pub fn in_place(self, font_size: f32) -> Self {
        Self {
            border: Border::InPlace,
            ..self.borderless(font_size)
        }
    }

    pub(crate) fn search(self, leading: f32, trailing: f32) -> Self {
        Self {
            border: Border::Search,
            leading,
            trailing,
            ..self
        }
    }

    pub(crate) fn with_trailing(self, trailing: f32) -> Self {
        Self { trailing, ..self }
    }

    pub(crate) fn replace_text(&mut self, text: &str) {
        let revision = self.editor.revision();
        self.editor.select_all();
        self.editor.insert(text);
        self.changed(revision);
    }

    pub(crate) fn clear(&mut self) {
        let revision = self.editor.revision();
        self.editor.clear();
        self.changed(revision);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.editor.is_empty()
    }

    fn height(&self) -> f32 {
        if matches!(self.border, Border::Field | Border::Search) {
            TEXT_FIELD_HEIGHT
        } else {
            (self.editor.font_size() * 1.5).ceil()
        }
    }

    pub fn text(&self) -> String {
        self.editor.text()
    }

    pub fn selection(&self) -> Range<usize> {
        self.editor.selection()
    }

    pub fn select(&mut self, range: Range<usize>) {
        self.editor.select(range);
    }

    fn inner_width(&self) -> f32 {
        (self.frame.width - self.leading - self.trailing).max(0.0)
    }

    fn text_origin(&self) -> Point {
        let (width, rtl) = self.editor.line_extent();
        let inner = self.inner_width();
        let shift = match rtl && width < inner {
            true => inner - width - CARET_ROOM,
            false => 0.0,
        };
        let height = self.editor.first_line_height();
        Point::new(
            self.frame.x + self.leading - self.scroll + shift,
            self.frame.y + ((self.frame.height - height) / 2.0).round(),
        )
    }

    fn reveal_caret(&mut self, cx: &mut Context) {
        let caret = self.editor.caret_rect(&mut cx.text);
        let inner = self.inner_width();
        let (width, _) = self.editor.line_extent();
        if caret.x - self.scroll < 0.0 {
            self.scroll = caret.x;
        } else if caret.x + CARET_ROOM - self.scroll > inner {
            self.scroll = caret.x + CARET_ROOM - inner;
        }
        self.scroll = self
            .scroll
            .clamp(0.0, (width + CARET_ROOM - inner).max(0.0))
            .round();
    }

    fn changed(&mut self, revision: u64) {
        if self.editor.revision() != revision {
            (self.on_change)(&self.editor.text());
        }
    }
}

impl Widget for TextField {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.editor.set_color(cx.palette.label_primary);
        self.editor.lay_out(&mut cx.text, WHOLE_LINE);
        self.placeholder_paragraph = (!self.placeholder.is_empty()).then(|| {
            cx.text.paragraph(
                &self.placeholder,
                TextStyle::new(self.editor.font_size(), cx.palette.label_muted),
                None,
                TextAlign::Start,
            )
        });
        Size::new(available.width, self.height())
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = Rect::new(frame.x, frame.y, frame.width, self.height());
        let content = Rect::new(
            self.frame.x + self.leading,
            f32::MIN / 2.0,
            self.inner_width(),
            f32::MAX,
        );
        if self
            .editor
            .autoscroll(&mut cx.text, self.text_origin(), content)
        {
            cx.animating.set(true);
        }
        self.editor.take_reveal();
        self.reveal_caret(cx);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let shape = Shape::rounded(self.frame, RADIUS);
        let focused = cx.is_focused(self.id);
        match self.border {
            Border::Field => {
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
            Border::InPlace => {
                let shape = Shape::rounded(self.frame, IN_PLACE_RADIUS);
                items.push(Item::Shadow {
                    shape,
                    offset: Point::new(0.0, 1.0),
                    blur_radius: 4.0,
                    color: Color::hex(0).with_alpha(60),
                });
                items.push(Item::Fill {
                    shape,
                    paint: cx.palette.surface_elevated.into(),
                });
                items.push(Item::Stroke {
                    shape,
                    width: 1.5,
                    paint: cx.palette.accent_color.into(),
                });
            }
            Border::Search => {
                let shape = Shape::rounded(self.frame, SEARCH_RADIUS);
                items.push(Item::Fill {
                    shape,
                    paint: cx.palette.label_muted.with_alpha(36).into(),
                });
                if focused {
                    super::focus_ring(cx, shape, items);
                }
            }
            Border::None => {}
        }
        let origin = self.text_origin();
        let mut content = Vec::new();
        if self.editor.is_empty()
            && let Some(placeholder) = &self.placeholder_paragraph
        {
            content.push(Item::Text {
                origin: Point::new(self.frame.x + self.leading, origin.y),
                paragraph: placeholder.clone(),
            });
        }
        self.editor
            .paint(cx, origin, WHOLE_LINE, focused, &mut content);
        let clip = Rect::new(
            self.frame.x + self.leading - CARET_ROOM,
            self.frame.y + 1.0,
            self.inner_width() + 2.0 * CARET_ROOM,
            (self.frame.height - 2.0).max(0.0),
        );
        items.push(Item::Clip {
            shape: Shape::rect(clip),
            items: content,
        });
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::TextInput);
        node.set_value(self.editor.text());
        node.set_placeholder(self.placeholder.clone());
        let runs = self.editor.runs(WHOLE_LINE);
        self.editor.describe(&mut node, self.id, &runs);
        node
    }

    fn parts(&self, _cx: &Context) -> Parts {
        let runs = self.editor.runs(WHOLE_LINE);
        let nodes = self.editor.run_nodes(self.id, &runs, self.text_origin());
        Parts {
            children: nodes.iter().map(|(id, _)| *id).collect(),
            nodes,
            focus: None,
        }
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        let revision = self.editor.revision();
        let handled = self.editor.handle(cx, event, self.text_origin());
        self.changed(revision);
        handled
    }

    fn perform(&mut self, cx: &mut Context, request: &ActionRequest) -> bool {
        let revision = self.editor.revision();
        let runs = self.editor.runs(WHOLE_LINE);
        let performed = self.editor.perform(self.id, &runs, request);
        self.editor.lay_out(&mut cx.text, WHOLE_LINE);
        self.changed(revision);
        performed
    }

    fn cursor(&self, _point: Point) -> Option<Cursor> {
        Some(Cursor::Text)
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }

    fn focused_by_keyboard(&mut self) {
        self.editor.select_all();
    }

    fn context_menu(&mut self, _cx: &mut Context, _point: Point) -> Option<Menu> {
        Some(self.editor.menu())
    }

    fn menu_action(&mut self, cx: &mut Context, action: &str) -> bool {
        let revision = self.editor.revision();
        let handled = self.editor.menu_action(cx, action);
        self.changed(revision);
        handled
    }

    fn text_input_area(&self) -> Option<Rect> {
        let origin = self.text_origin();
        let area = self.editor.input_area();
        Some(Rect::new(
            origin.x + area.x,
            origin.y + area.y,
            area.width,
            area.height,
        ))
    }
}
