use accesskit::{ActionRequest, Node, Role};

use super::control::paint_field;
use super::editor::Editor;
use super::{Context, Cursor, DragAction, Event, Menu, Parts, ScrollState, Size, Widget, WidgetId};
use bande_graphics::{Item, Paragraph, Point, Rect, Shape, TextAlign, TextStyle};

const FONT_SIZE: f32 = 13.0;
const HORIZONTAL_PADDING: f32 = 8.0;
const VERTICAL_PADDING: f32 = 5.0;
const CARET_ROOM: f32 = 1.0;

pub struct TextView {
    id: WidgetId,
    placeholder: String,
    placeholder_paragraph: Option<Paragraph>,
    editor: Editor,
    on_change: Box<dyn FnMut(&str)>,
    scroll: ScrollState,
    offset: f32,
    visible_lines: Option<u16>,
    borderless: bool,
    frame: Rect,
}

impl TextView {
    pub fn new(placeholder: &str, text: &str, on_change: impl FnMut(&str) + 'static) -> Self {
        let mut editor = Editor::new(text, FONT_SIZE, true);
        editor.select(0..0);
        Self {
            id: WidgetId::next(),
            placeholder: placeholder.to_owned(),
            placeholder_paragraph: None,
            editor,
            on_change: Box::new(on_change),
            scroll: ScrollState::default(),
            offset: 0.0,
            visible_lines: None,
            borderless: false,
            frame: Rect::default(),
        }
    }

    pub fn read_only(mut self) -> Self {
        self.editor.set_read_only(true);
        self
    }

    pub fn borderless(self) -> Self {
        Self {
            borderless: true,
            ..self
        }
    }

    pub fn visible_lines(self, lines: u16) -> Self {
        Self {
            visible_lines: Some(lines),
            ..self
        }
    }

    pub fn text(&self) -> String {
        self.editor.text()
    }

    pub fn set_text(&mut self, text: &str) {
        self.editor.set_text(text);
        self.editor.select(0..0);
    }

    pub fn selection(&self) -> std::ops::Range<usize> {
        self.editor.selection()
    }

    pub fn select(&mut self, range: std::ops::Range<usize>) {
        self.editor.select(range);
    }

    pub fn offset(&self) -> f32 {
        self.offset
    }

    fn content(&self) -> Rect {
        Rect::new(
            self.frame.x + HORIZONTAL_PADDING,
            self.frame.y + VERTICAL_PADDING,
            (self.frame.width - 2.0 * HORIZONTAL_PADDING - CARET_ROOM).max(0.0),
            (self.frame.height - 2.0 * VERTICAL_PADDING).max(0.0),
        )
    }

    fn origin(&self) -> Point {
        let content = self.content();
        Point::new(content.x, content.y - self.offset)
    }

    fn visible(&self) -> std::ops::Range<f32> {
        self.offset..self.offset + self.content().height
    }

    fn changed(&mut self, revision: u64) {
        if self.editor.revision() != revision {
            (self.on_change)(&self.editor.text());
        }
    }

    fn sync(&mut self, cx: &mut Context) {
        let content = self.content();
        self.editor.set_width(Some(content.width));
        self.editor.set_page(content.height);
        self.offset = self.scroll.offset(cx);
        self.editor.lay_out(&mut cx.text, self.visible());
        self.scroll.set_extent(self.editor.height(), content.height);
        if self.editor.autoscroll(&mut cx.text, self.origin(), content) {
            cx.animating.set(true);
        }
        if self.editor.take_reveal() {
            let caret = self.editor.caret_rect(&mut cx.text);
            self.scroll.set_extent(self.editor.height(), content.height);
            self.scroll.follow(cx, caret.y, caret.y + caret.height);
        }
        let offset = self.scroll.offset(cx);
        if offset != self.offset {
            self.offset = offset;
            self.editor.lay_out(&mut cx.text, self.visible());
            self.scroll.set_extent(self.editor.height(), content.height);
        }
    }
}

impl Widget for TextView {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.editor.set_scale(cx.text.scale());
        self.editor.set_color(cx.palette.ink);
        self.placeholder_paragraph = (!self.placeholder.is_empty()).then(|| {
            cx.text.paragraph(
                &self.placeholder,
                TextStyle::new(FONT_SIZE, cx.palette.ink3),
                Some((available.width - 2.0 * HORIZONTAL_PADDING).max(0.0)),
                TextAlign::Start,
            )
        });
        let height = match self.visible_lines {
            Some(lines) => {
                (f32::from(lines) * self.editor.first_line_height() + 2.0 * VERTICAL_PADDING).ceil()
            }
            None => available.height,
        };
        Size::new(available.width, height.min(available.height))
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        self.sync(cx);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let focused = cx.is_focused(self.id);
        if !self.borderless {
            paint_field(cx, self.frame, focused, items);
        }
        let content = self.content();
        let mut inner = Vec::new();
        if self.editor.is_empty()
            && let Some(placeholder) = &self.placeholder_paragraph
        {
            inner.push(Item::Text {
                origin: Point::new(content.x, content.y),
                paragraph: placeholder.clone(),
            });
        }
        self.editor
            .paint(cx, self.origin(), self.visible(), focused, &mut inner);
        items.push(Item::Clip {
            shape: Shape::rect(Rect::new(
                self.frame.x + 1.0,
                self.frame.y + 1.0,
                (self.frame.width - 2.0).max(0.0),
                (self.frame.height - 2.0).max(0.0),
            )),
            items: inner,
        });
        self.scroll.paint_scrollbar(cx, self.frame, items);
    }

    fn access(&self, cx: &Context) -> Node {
        let mut node = Node::new(Role::MultilineTextInput);
        node.set_placeholder(self.placeholder.clone());
        let runs = self.editor.runs(self.visible());
        self.editor.describe(&mut node, self.id, &runs);
        self.scroll.describe(cx, &mut node);
        node
    }

    fn parts(&self, _cx: &Context) -> Parts {
        let runs = self.editor.runs(self.visible());
        let nodes = self.editor.run_nodes(self.id, &runs, self.origin());
        Parts {
            children: nodes.iter().map(|(id, _)| *id).collect(),
            nodes,
            focus: None,
        }
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        if let Event::Scroll(scroll) = event {
            return self.scroll.scroll(cx, scroll);
        }
        if self.scroll.pointer(cx, self.frame, event) {
            return true;
        }
        let revision = self.editor.revision();
        let handled = self.editor.handle(cx, event, self.origin());
        self.changed(revision);
        if handled {
            self.sync(cx);
        }
        handled
    }

    fn drop_target(&mut self, cx: &mut Context, point: Option<Point>) -> Option<DragAction> {
        let origin = self.origin();
        let local = point.map(|point| Point::new(point.x - origin.x, point.y - origin.y));
        self.editor.drop_target(&mut cx.text, local)
    }

    fn drop_text(&mut self, cx: &mut Context, point: Point, text: &str, action: DragAction) {
        let origin = self.origin();
        let revision = self.editor.revision();
        self.editor.drop_text(
            &mut cx.text,
            Point::new(point.x - origin.x, point.y - origin.y),
            text,
            action,
        );
        self.changed(revision);
        self.sync(cx);
    }

    fn drag_ended(&mut self, action: Option<DragAction>) {
        let revision = self.editor.revision();
        self.editor.drag_ended(action);
        self.changed(revision);
    }

    fn perform(&mut self, cx: &mut Context, request: &ActionRequest) -> bool {
        if self.scroll.perform(cx, request) {
            return true;
        }
        let revision = self.editor.revision();
        let runs = self.editor.runs(self.visible());
        let performed = self.editor.perform(self.id, &runs, request);
        self.changed(revision);
        self.sync(cx);
        performed
    }

    fn cursor(&self, point: Point) -> Option<Cursor> {
        match self.scroll.claims(self.frame, point) {
            true => None,
            false => Some(Cursor::Text),
        }
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }

    fn context_menu(&mut self, _cx: &mut Context, _point: Point) -> Option<Menu> {
        Some(self.editor.menu())
    }

    fn menu_action(&mut self, cx: &mut Context, action: &str) -> bool {
        let revision = self.editor.revision();
        let handled = self.editor.menu_action(cx, action);
        self.changed(revision);
        self.sync(cx);
        handled
    }

    fn text_input_area(&self) -> Option<Rect> {
        let origin = self.origin();
        let area = self.editor.input_area();
        Some(Rect::new(
            origin.x + area.x,
            origin.y + area.y,
            area.width,
            area.height,
        ))
    }
}
