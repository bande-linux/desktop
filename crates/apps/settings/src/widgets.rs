use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::time::Instant;

use accesskit::{Action, ActionRequest, Node, Role};
use unbound_toolkit::ui::{
    Animated, Context, Event, Key, Size, Spring, Widget, WidgetId, focus_ring,
};
use unbound_toolkit::{
    Color, Image, Item, Paragraph, Point, Radius, Rect, SHEET_INSET, Scheme, Shape, Spacing,
    TextAlign, TextRole, TextSize, TextStyle,
};

const SIDEBAR_WIDTH: f32 = 220.0;
const SIDEBAR_GROWTH: f32 = 60.0;

pub fn sidebar_width(text: TextSize) -> f32 {
    (SIDEBAR_WIDTH + SIDEBAR_GROWTH * (text.scale() - 1.0)).round()
}
const MARGIN: f32 = Spacing::M as f32;
const SIDEBAR_INSET: f32 = Spacing::S as f32;
const BODY_SIZE: f32 = 13.0;
const CAPTION_SIZE: f32 = 11.0;
const SIDEBAR_ROW_HEIGHT: f32 = 28.0;
const SIDEBAR_SYMBOL_INSET: f32 = 8.0;
pub(crate) const SIDEBAR_SYMBOL: f32 = 16.0;
const SIDEBAR_TEXT_INSET: f32 = SIDEBAR_SYMBOL_INSET + SIDEBAR_SYMBOL + 8.0;
const SIDEBAR_LABEL_GAP: f32 = 10.0;
const HEADER_TILE: f32 = 40.0;
pub(crate) const HEADER_SYMBOL: f32 = 24.0;
const HEADER_GAP: f32 = Spacing::S as f32;
pub(crate) const NOTICE_SYMBOL: f32 = 48.0;
const NOTICE_PADDING: f32 = Spacing::XL as f32;
const ROW_PADDING: f32 = Spacing::M as f32;
const ROW_MIN_HEIGHT: f32 = 44.0;
const ROW_GAP: f32 = Spacing::M as f32;
const CONTROL_WIDTH: f32 = 320.0;
const SCROLL_BAR_WIDTH: f32 = 6.0;
const UNBOUNDED: f32 = 1.0e6;

fn contains(frame: Rect, point: Point) -> bool {
    point.x >= frame.x
        && point.y >= frame.y
        && point.x < frame.x + frame.width
        && point.y < frame.y + frame.height
}

fn is_activation(event: &Event) -> bool {
    matches!(
        event,
        Event::Key {
            key: Key::Space | Key::Enter,
            ..
        }
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Primary,
    Muted,
    Faint,
    Danger,
}

pub struct Text {
    text: String,
    highlights: Vec<Range<usize>>,
    size: f32,
    weight: f32,
    role: Option<TextRole>,
    tone: Tone,
    paragraph: Option<Paragraph>,
    measured: Size,
}

impl Text {
    pub fn new(text: &str, tone: Tone) -> Self {
        Self {
            text: text.to_owned(),
            highlights: Vec::new(),
            size: BODY_SIZE,
            weight: TextStyle::REGULAR,
            role: None,
            tone,
            paragraph: None,
            measured: Size::default(),
        }
    }

    pub fn highlighted(mut self, highlights: Vec<Range<usize>>) -> Self {
        let valid = highlights
            .iter()
            .all(|range| self.text.get(range.clone()).is_some());
        self.highlights = if valid { highlights } else { Vec::new() };
        self
    }

    pub fn caption(mut self) -> Self {
        self.size = CAPTION_SIZE;
        self
    }

    pub fn semibold(mut self) -> Self {
        self.weight = TextStyle::SEMIBOLD;
        self
    }

    pub fn role(mut self, role: TextRole) -> Self {
        self.role = Some(role);
        self
    }

    pub fn label(text: &str) -> Self {
        Self::new(text, Tone::Faint).role(TextRole::Label)
    }

    pub fn measure(&mut self, cx: &mut Context) -> Size {
        self.measure_within(cx, None)
    }

    pub fn measure_within(&mut self, cx: &mut Context, max_width: Option<f32>) -> Size {
        let color = match self.tone {
            Tone::Primary => cx.palette.ink,
            Tone::Muted => cx.palette.ink2,
            Tone::Faint => cx.palette.ink3,
            Tone::Danger => cx.palette.danger,
        };
        let style = match self.role {
            Some(role) => TextStyle::role(role, color),
            None => TextStyle::new(self.size, color).weight(self.weight),
        };
        let emphasis: Vec<_> = self
            .highlights
            .iter()
            .map(|range| (range.clone(), TextStyle::SEMIBOLD))
            .collect();
        let paragraph =
            cx.text
                .emphasized(&self.text, style, &emphasis, max_width, TextAlign::Start);
        self.measured = Size::new(paragraph.width().ceil(), paragraph.height().ceil());
        self.paragraph = Some(paragraph);
        self.measured
    }

    pub fn size(&self) -> Size {
        self.measured
    }

    pub fn paint(&self, cx: &Context, origin: Point, items: &mut Vec<Item>) {
        let Some(paragraph) = &self.paragraph else {
            return;
        };
        let at = Point::new(origin.x.round(), origin.y.round());
        for range in &self.highlights {
            for rect in paragraph.span_rects(range.clone()) {
                items.push(Item::Fill {
                    shape: Shape::rounded(
                        Rect::new(
                            at.x + rect.x - 1.0,
                            at.y + rect.y,
                            rect.width + 2.0,
                            rect.height,
                        ),
                        Radius::XS,
                    ),
                    paint: cx.palette.accent_soft.into(),
                });
            }
        }
        items.push(Item::Text {
            origin: at,
            paragraph: paragraph.clone(),
        });
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }
}

pub struct Root {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    sizes: Vec<Size>,
    inset: f32,
    sidebar: f32,
    frame: Rect,
}

impl Root {
    pub const PANES: usize = 1;
    pub const CONTENT: usize = 2;

    pub fn new(search: Box<dyn Widget>, panes: Box<dyn Widget>, content: Box<dyn Widget>) -> Self {
        Self {
            id: WidgetId::next(),
            children: vec![search, panes, content],
            sizes: Vec::new(),
            inset: SHEET_INSET as f32,
            sidebar: SIDEBAR_WIDTH,
            frame: Rect::default(),
        }
    }

    pub fn set_inset(&mut self, inset: f32) {
        self.inset = inset;
    }

    fn sheet(&self) -> Rect {
        Rect::new(
            self.sidebar,
            self.frame.y,
            (self.frame.width - self.sidebar - self.inset).max(0.0),
            (self.frame.height - self.inset).max(0.0),
        )
    }
}

impl Widget for Root {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.sidebar = sidebar_width(cx.text.size());
        let sidebar = Size::new(self.sidebar - 2.0 * SIDEBAR_INSET, available.height);
        let content = Size::new(
            (available.width - self.sidebar - self.inset).max(0.0),
            (available.height - self.inset).max(0.0),
        );
        self.sizes = self
            .children
            .iter_mut()
            .enumerate()
            .map(|(index, child)| {
                child.measure(
                    cx,
                    if index == Self::CONTENT {
                        content
                    } else {
                        sidebar
                    },
                )
            })
            .collect();
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let search_height = self.sizes.first().map_or(0.0, |size| size.height);
        let width = self.sidebar - 2.0 * SIDEBAR_INSET;
        let frames = [
            Rect::new(SIDEBAR_INSET, 0.0, width, search_height),
            Rect::new(
                SIDEBAR_INSET,
                MARGIN + search_height,
                width,
                (frame.height - 2.0 * MARGIN - search_height).max(0.0),
            ),
            self.sheet(),
        ];
        for (child, child_frame) in self.children.iter_mut().zip(frames) {
            child.place(cx, child_frame);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for (index, child) in self.children.iter().enumerate() {
            if index != Self::CONTENT {
                child.paint(cx, items);
                continue;
            }
            let radius = match self.inset > 0.0 {
                true => Radius::concentric(Radius::XL, self.inset),
                false => 0.0,
            };
            let mut content = Vec::new();
            child.paint(cx, &mut content);
            items.push(Item::Clip {
                shape: Shape::rounded(self.sheet(), radius),
                items: content,
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::Window)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub struct Column {
    id: WidgetId,
    spacing: f32,
    padding: f32,
    children: Vec<Box<dyn Widget>>,
    sizes: Vec<Size>,
    frame: Rect,
}

impl Column {
    pub fn new(spacing: f32, padding: f32, children: Vec<Box<dyn Widget>>) -> Self {
        Self {
            id: WidgetId::next(),
            spacing,
            padding,
            children,
            sizes: Vec::new(),
            frame: Rect::default(),
        }
    }
}

impl Widget for Column {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let inner = Size::new((available.width - 2.0 * self.padding).max(0.0), UNBOUNDED);
        self.sizes = self
            .children
            .iter_mut()
            .map(|child| child.measure(cx, inner))
            .collect();
        let height = self.sizes.iter().map(|size| size.height).sum::<f32>()
            + self.spacing * self.sizes.len().saturating_sub(1) as f32
            + 2.0 * self.padding;
        Size::new(available.width, height)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let mut y = frame.y + self.padding;
        for (child, size) in self.children.iter_mut().zip(&self.sizes) {
            child.place(
                cx,
                Rect::new(
                    frame.x + self.padding,
                    y,
                    frame.width - 2.0 * self.padding,
                    size.height,
                ),
            );
            y += size.height + self.spacing;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::GenericContainer)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub struct Caption {
    id: WidgetId,
    text: Text,
    inset: f32,
    frame: Rect,
}

impl Caption {
    pub fn new(text: Text) -> Self {
        Self {
            id: WidgetId::next(),
            text,
            inset: 0.0,
            frame: Rect::default(),
        }
    }

    pub fn inset(mut self, inset: f32) -> Self {
        self.inset = inset;
        self
    }
}

impl Widget for Caption {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        Size::new(available.width, self.text.measure(cx).height)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        self.text.paint(
            cx,
            Point::new(self.frame.x + self.inset, self.frame.y),
            items,
        );
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Label);
        node.set_value(self.text.as_str().to_owned());
        node
    }
}

pub struct SidebarLabel {
    id: WidgetId,
    text: Text,
    frame: Rect,
}

impl SidebarLabel {
    pub fn new(label: &str) -> Self {
        Self {
            id: WidgetId::next(),
            text: Text::label(label),
            frame: Rect::default(),
        }
    }
}

impl Widget for SidebarLabel {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let height = self.text.measure(cx).height;
        Size::new(available.width, height + SIDEBAR_LABEL_GAP)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let y = self.frame.y + self.frame.height - self.text.size().height - 2.0;
        self.text.paint(
            cx,
            Point::new(self.frame.x + SIDEBAR_SYMBOL_INSET, y),
            items,
        );
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Heading);
        node.set_label(self.text.as_str().to_owned());
        node
    }
}

pub struct SidebarRow {
    id: WidgetId,
    text: Text,
    symbol: Option<Image>,
    selected: bool,
    on_select: Box<dyn FnMut()>,
    on_step: Box<dyn FnMut(isize)>,
    frame: Rect,
}

impl SidebarRow {
    pub fn new(
        text: Text,
        symbol: Option<Image>,
        selected: bool,
        on_select: impl FnMut() + 'static,
        on_step: impl FnMut(isize) + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            text,
            symbol,
            selected,
            on_select: Box::new(on_select),
            on_step: Box::new(on_step),
            frame: Rect::default(),
        }
    }
}

fn symbol_rect(center: Point, size: f32) -> Rect {
    Rect::new(
        (center.x - size / 2.0).round(),
        (center.y - size / 2.0).round(),
        size,
        size,
    )
}

pub struct Header {
    id: WidgetId,
    symbol: Option<Image>,
    title: Text,
    summary: Text,
    children: Vec<Box<dyn Widget>>,
    control: Size,
    frame: Rect,
}

impl Header {
    pub fn new(
        symbol: Option<Image>,
        title: Text,
        summary: &str,
        master: Option<Box<dyn Widget>>,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            symbol,
            title,
            summary: Text::new(summary, Tone::Muted).role(TextRole::Callout),
            children: master.into_iter().collect(),
            control: Size::default(),
            frame: Rect::default(),
        }
    }

    fn text_height(&self) -> f32 {
        self.title.size().height + self.summary.size().height
    }
}

impl Widget for Header {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.control = self
            .children
            .iter_mut()
            .map(|child| child.measure(cx, Size::new(CONTROL_WIDTH, UNBOUNDED)))
            .next()
            .unwrap_or_default();
        let beside_control = match self.control.width > 0.0 {
            true => self.control.width + ROW_PADDING + ROW_GAP,
            false => 0.0,
        };
        let text_width =
            Some((available.width - HEADER_TILE - Spacing::M as f32 - beside_control).max(0.0));
        self.title.measure_within(cx, text_width);
        self.summary.measure_within(cx, text_width);
        let height = HEADER_TILE.max(self.text_height()).max(self.control.height);
        Size::new(available.width, height.ceil() + HEADER_GAP)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let size = self.control;
        let height = frame.height - HEADER_GAP;
        for child in &mut self.children {
            child.place(
                cx,
                Rect::new(
                    (frame.x + frame.width - ROW_PADDING - size.width).round(),
                    (frame.y + (height - size.height) / 2.0).round(),
                    size.width,
                    size.height,
                ),
            );
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let height = self.frame.height - HEADER_GAP;
        let tile = Rect::new(
            self.frame.x,
            (self.frame.y + (height - HEADER_TILE) / 2.0).round(),
            HEADER_TILE,
            HEADER_TILE,
        );
        items.push(Item::Fill {
            shape: Shape::rounded(tile, Radius::L),
            paint: cx.palette.well.into(),
        });
        if let Some(symbol) = &self.symbol {
            items.push(Item::Image {
                rect: symbol_rect(
                    Point::new(tile.x + HEADER_TILE / 2.0, tile.y + HEADER_TILE / 2.0),
                    HEADER_SYMBOL,
                ),
                image: symbol.clone(),
            });
        }
        let x = tile.x + HEADER_TILE + Spacing::M as f32;
        let y = self.frame.y + (height - self.text_height()) / 2.0;
        self.title.paint(cx, Point::new(x, y), items);
        self.summary
            .paint(cx, Point::new(x, y + self.title.size().height), items);
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Heading);
        node.set_label(self.title.as_str().to_owned());
        node.set_value(self.summary.as_str().to_owned());
        node
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub struct Notice {
    id: WidgetId,
    symbol: Option<Image>,
    heading: Text,
    line: Text,
    frame: Rect,
}

impl Notice {
    pub fn new(symbol: Option<Image>, heading: &str, line: &str) -> Self {
        Self {
            id: WidgetId::next(),
            symbol,
            heading: Text::new(heading, Tone::Primary).role(TextRole::Heading),
            line: Text::new(line, Tone::Muted).role(TextRole::Callout),
            frame: Rect::default(),
        }
    }
}

impl Widget for Notice {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let text = self.heading.measure(cx).height + self.line.measure(cx).height;
        Size::new(
            available.width,
            NOTICE_PADDING * 2.0 + NOTICE_SYMBOL + Spacing::M as f32 + text,
        )
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let center = self.frame.x + self.frame.width / 2.0;
        let top = self.frame.y + NOTICE_PADDING;
        if let Some(symbol) = &self.symbol {
            items.push(Item::Image {
                rect: symbol_rect(Point::new(center, top + NOTICE_SYMBOL / 2.0), NOTICE_SYMBOL),
                image: symbol.clone(),
            });
        }
        let y = top + NOTICE_SYMBOL + Spacing::M as f32;
        let heading = self.heading.size();
        self.heading
            .paint(cx, Point::new(center - heading.width / 2.0, y), items);
        self.line.paint(
            cx,
            Point::new(center - self.line.size().width / 2.0, y + heading.height),
            items,
        );
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Label);
        node.set_label(self.heading.as_str().to_owned());
        node.set_value(self.line.as_str().to_owned());
        node
    }
}

impl Widget for SidebarRow {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.text.measure(cx);
        Size::new(available.width, SIDEBAR_ROW_HEIGHT + cx.grown_line())
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let shape = Shape::rounded(self.frame, Radius::S);
        if self.selected {
            items.push(Item::Fill {
                shape,
                paint: cx.palette.well.into(),
            });
        }
        if cx.is_focused(self.id) {
            focus_ring(cx, shape, items);
        }
        let middle = self.frame.y + self.frame.height / 2.0;
        if let Some(symbol) = &self.symbol {
            items.push(Item::Image {
                rect: symbol_rect(
                    Point::new(
                        self.frame.x + SIDEBAR_SYMBOL_INSET + SIDEBAR_SYMBOL / 2.0,
                        middle,
                    ),
                    SIDEBAR_SYMBOL,
                ),
                image: symbol.clone(),
            });
        }
        let height = self.text.size().height;
        self.text.paint(
            cx,
            Point::new(self.frame.x + SIDEBAR_TEXT_INSET, middle - height / 2.0),
            items,
        );
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::ListItem);
        node.set_label(self.text.as_str().to_owned());
        node.set_selected(self.selected);
        node.add_action(Action::Click);
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => (self.on_select)(),
            Event::Key { key: Key::Up, .. } => (self.on_step)(-1),
            Event::Key { key: Key::Down, .. } => (self.on_step)(1),
            event if is_activation(event) => (self.on_select)(),
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action == Action::Click {
            (self.on_select)();
            return true;
        }
        false
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }
}

fn focused_frame(widget: &dyn Widget, cx: &Context) -> Option<Rect> {
    if cx.is_focused(widget.id()) {
        return Some(widget.frame());
    }
    widget
        .children()
        .iter()
        .find_map(|child| focused_frame(child.as_ref(), cx))
}

pub struct Scroll {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    offset: Rc<Cell<f32>>,
    content_height: f32,
    frame: Rect,
}

impl Scroll {
    pub fn new(child: Box<dyn Widget>, offset: Rc<Cell<f32>>) -> Self {
        Self {
            id: WidgetId::next(),
            children: vec![child],
            offset,
            content_height: 0.0,
            frame: Rect::default(),
        }
    }

    fn place_child(&mut self, cx: &mut Context) {
        let limit = (self.content_height - self.frame.height).max(0.0);
        let offset = self.offset.get().clamp(0.0, limit);
        self.offset.set(offset);
        let frame = Rect::new(
            self.frame.x,
            self.frame.y - offset,
            self.frame.width,
            self.content_height,
        );
        for child in &mut self.children {
            child.place(cx, frame);
        }
    }
}

impl Widget for Scroll {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.content_height = self
            .children
            .iter_mut()
            .map(|child| {
                child
                    .measure(cx, Size::new(available.width, UNBOUNDED))
                    .height
            })
            .fold(0.0, f32::max);
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        self.place_child(cx);
        let focused = self
            .children
            .iter()
            .find_map(|child| focused_frame(child.as_ref(), cx));
        if let Some(focused) = focused {
            let top = self.frame.y + MARGIN;
            let bottom = self.frame.y + self.frame.height - MARGIN;
            let shift = if focused.y < top {
                focused.y - top
            } else if focused.y + focused.height > bottom {
                (focused.y + focused.height - bottom).min(focused.y - top)
            } else {
                0.0
            };
            if shift != 0.0 {
                self.offset.set(self.offset.get() + shift);
                self.place_child(cx);
            }
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let mut clipped = Vec::new();
        for child in &self.children {
            child.paint(cx, &mut clipped);
        }
        items.push(Item::Clip {
            shape: Shape::rect(self.frame),
            items: clipped,
        });
        if self.content_height > self.frame.height {
            let visible = self.frame.height / self.content_height;
            let height = (self.frame.height * visible).max(24.0);
            let travel = self.frame.height - height;
            let limit = self.content_height - self.frame.height;
            let y = self.frame.y + travel * self.offset.get() / limit;
            items.push(Item::Fill {
                shape: Shape::rounded(
                    Rect::new(
                        self.frame.x + self.frame.width - SCROLL_BAR_WIDTH - 3.0,
                        y + 3.0,
                        SCROLL_BAR_WIDTH,
                        height - 6.0,
                    ),
                    SCROLL_BAR_WIDTH / 2.0,
                ),
                paint: cx.palette.ink3.into(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::ScrollView)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub struct Card {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    sizes: Vec<Size>,
    frame: Rect,
}

impl Card {
    pub fn new(children: Vec<Box<dyn Widget>>) -> Self {
        Self {
            id: WidgetId::next(),
            children,
            sizes: Vec::new(),
            frame: Rect::default(),
        }
    }
}

impl Widget for Card {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.sizes = self
            .children
            .iter_mut()
            .map(|child| child.measure(cx, Size::new(available.width, UNBOUNDED)))
            .collect();
        Size::new(
            available.width,
            self.sizes.iter().map(|size| size.height).sum(),
        )
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let mut y = frame.y;
        for (child, size) in self.children.iter_mut().zip(&self.sizes) {
            child.place(cx, Rect::new(frame.x, y, frame.width, size.height));
            y += size.height;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::Group)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub struct Row {
    id: WidgetId,
    label: Text,
    detail: Option<Text>,
    highlighted: bool,
    on_activate: Option<Box<dyn FnMut()>>,
    children: Vec<Box<dyn Widget>>,
    control: Size,
    pressed: bool,
    frame: Rect,
}

impl Row {
    pub fn new(label: Text, control: Option<Box<dyn Widget>>) -> Self {
        Self {
            id: WidgetId::next(),
            label,
            detail: None,
            highlighted: false,
            on_activate: None,
            children: control.into_iter().collect(),
            control: Size::default(),
            pressed: false,
            frame: Rect::default(),
        }
    }

    pub fn detail(mut self, detail: Text) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn highlighted(mut self, highlighted: bool) -> Self {
        self.highlighted = highlighted;
        self
    }

    pub fn on_activate(mut self, on_activate: impl FnMut() + 'static) -> Self {
        self.on_activate = Some(Box::new(on_activate));
        self
    }

    fn activate(&mut self) {
        if let Some(on_activate) = &mut self.on_activate {
            on_activate();
        }
    }

    fn text_height(&self) -> f32 {
        self.label.size().height
            + self
                .detail
                .as_ref()
                .map_or(0.0, |detail| detail.size().height + 2.0)
    }
}

impl Widget for Row {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let control_width = CONTROL_WIDTH.min(available.width / 2.0);
        self.control = self
            .children
            .iter_mut()
            .map(|child| child.measure(cx, Size::new(control_width, UNBOUNDED)))
            .next()
            .unwrap_or_default();
        let beside_control = match self.control.width > 0.0 {
            true => self.control.width + ROW_GAP,
            false => 0.0,
        };
        let text_width = Some((available.width - 2.0 * ROW_PADDING - beside_control).max(0.0));
        self.label.measure_within(cx, text_width);
        if let Some(detail) = &mut self.detail {
            detail.measure_within(cx, text_width);
        }
        let height = (self.text_height().max(self.control.height) + 2.0 * Spacing::S as f32)
            .max(ROW_MIN_HEIGHT);
        Size::new(available.width, height.ceil())
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let size = self.control;
        for child in &mut self.children {
            child.place(
                cx,
                Rect::new(
                    (frame.x + frame.width - ROW_PADDING - size.width).round(),
                    (frame.y + (frame.height - size.height) / 2.0).round(),
                    size.width,
                    size.height,
                ),
            );
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let shape = Shape::rounded(self.frame, Radius::S);
        if self.highlighted {
            items.push(Item::Fill {
                shape,
                paint: cx.palette.accent_soft.into(),
            });
        }
        if self.pressed {
            items.push(Item::Fill {
                shape,
                paint: cx.palette.well.into(),
            });
        }
        if cx.is_focused(self.id) {
            focus_ring(cx, shape, items);
        }
        let x = self.frame.x + ROW_PADDING;
        let y = self.frame.y + (self.frame.height - self.text_height()) / 2.0;
        self.label.paint(cx, Point::new(x, y), items);
        if let Some(detail) = &self.detail {
            detail.paint(cx, Point::new(x, y + self.label.size().height + 2.0), items);
        }
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(if self.on_activate.is_some() {
            Role::Button
        } else {
            Role::Row
        });
        let mut label = self.label.as_str().to_owned();
        if let Some(detail) = &self.detail {
            label = format!("{label}, {}", detail.as_str());
        }
        node.set_label(label);
        if self.on_activate.is_some() {
            node.add_action(Action::Click);
        }
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        if self.on_activate.is_none() {
            return false;
        }
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => self.pressed = true,
            Event::PointerUp(pointer) if pointer.is_primary() => {
                let activate = self.pressed && contains(self.frame, pointer.position);
                self.pressed = false;
                if activate {
                    self.activate();
                }
            }
            event if is_activation(event) => self.activate(),
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action == Action::Click && self.on_activate.is_some() {
            self.activate();
            return true;
        }
        false
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        self.on_activate.is_some()
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub struct Captioned {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    low: Text,
    high: Text,
    control: Size,
    frame: Rect,
}

impl Captioned {
    pub fn new(control: Box<dyn Widget>, low: &str, high: &str) -> Self {
        Self {
            id: WidgetId::next(),
            children: vec![control],
            low: Text::new(low, Tone::Muted).caption(),
            high: Text::new(high, Tone::Muted).caption(),
            control: Size::default(),
            frame: Rect::default(),
        }
    }
}

impl Widget for Captioned {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.control = self
            .children
            .iter_mut()
            .map(|child| child.measure(cx, available))
            .next()
            .unwrap_or_default();
        let captions = self
            .low
            .measure(cx)
            .height
            .max(self.high.measure(cx).height);
        Size::new(available.width, self.control.height + 2.0 + captions)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let control = Rect::new(frame.x, frame.y, frame.width, self.control.height);
        for child in &mut self.children {
            child.place(cx, control);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for child in &self.children {
            child.paint(cx, items);
        }
        let y = self.frame.y + self.control.height + 2.0;
        self.low.paint(cx, Point::new(self.frame.x, y), items);
        self.high.paint(
            cx,
            Point::new(self.frame.x + self.frame.width - self.high.size().width, y),
            items,
        );
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::GenericContainer)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

const BAR_WIDTH: f32 = 3.0;
const BAR_GAP: f32 = 2.0;
const BARS: u8 = 4;
const SIGNAL_HEIGHT: f32 = 12.0;
const LOCK_WIDTH: f32 = 9.0;

pub struct Signal {
    id: WidgetId,
    strength: u8,
    secured: bool,
    frame: Rect,
}

impl Signal {
    pub fn new(strength: u8, secured: bool) -> Self {
        Self {
            id: WidgetId::next(),
            strength,
            secured,
            frame: Rect::default(),
        }
    }

    fn bars(&self) -> u8 {
        (self.strength.min(100) + 12) / 25
    }
}

impl Widget for Signal {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, _available: Size) -> Size {
        let bars = f32::from(BARS) * (BAR_WIDTH + BAR_GAP);
        Size::new(bars + LOCK_WIDTH + BAR_GAP, SIGNAL_HEIGHT)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let mut x = self.frame.x;
        if self.secured {
            let body = Rect::new(x, self.frame.y + 5.0, LOCK_WIDTH, 7.0);
            items.push(Item::Stroke {
                shape: Shape::rounded(Rect::new(x + 2.0, self.frame.y + 1.0, 5.0, 8.0), 2.5),
                width: 1.5,
                paint: cx.palette.ink2.into(),
            });
            items.push(Item::Fill {
                shape: Shape::rounded(body, 1.5),
                paint: cx.palette.ink2.into(),
            });
        }
        x += LOCK_WIDTH + BAR_GAP;
        for bar in 0..BARS {
            let height = SIGNAL_HEIGHT * f32::from(bar + 1) / f32::from(BARS);
            let color = if bar < self.bars() {
                cx.palette.ink
            } else {
                cx.palette.hairline
            };
            items.push(Item::Fill {
                shape: Shape::rounded(
                    Rect::new(x, self.frame.y + SIGNAL_HEIGHT - height, BAR_WIDTH, height),
                    1.0,
                ),
                paint: color.into(),
            });
            x += BAR_WIDTH + BAR_GAP;
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Meter);
        node.set_numeric_value(f64::from(self.strength));
        node.set_min_numeric_value(0.0);
        node.set_max_numeric_value(100.0);
        node
    }
}

pub struct Check {
    id: WidgetId,
    frame: Rect,
}

impl Check {
    pub fn new() -> Self {
        Self {
            id: WidgetId::next(),
            frame: Rect::default(),
        }
    }
}

impl Widget for Check {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, _available: Size) -> Size {
        Size::new(14.0, 14.0)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let Rect { x, y, .. } = self.frame;
        let stroke = |from: Point, to: Point, items: &mut Vec<Item>| {
            let steps = 8;
            for step in 0..=steps {
                let t = step as f32 / steps as f32;
                let at = Point::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
                items.push(Item::Fill {
                    shape: Shape::rounded(Rect::new(at.x - 1.0, at.y - 1.0, 2.0, 2.0), 1.0),
                    paint: cx.palette.accent.into(),
                });
            }
        };
        stroke(
            Point::new(x + 2.0, y + 7.5),
            Point::new(x + 5.5, y + 11.0),
            items,
        );
        stroke(
            Point::new(x + 5.5, y + 11.0),
            Point::new(x + 12.0, y + 3.0),
            items,
        );
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::Image)
    }
}

const THUMBNAIL: Size = Size::new(144.0, 90.0);
const THUMBNAIL_GAP: f32 = Spacing::M as f32;
const THUMBNAIL_RADIUS: f32 = Radius::M;
const THUMBNAIL_LABEL_GAP: f32 = 6.0;

pub struct Thumbnail {
    pub label: String,
    pub image: Option<Image>,
}

pub struct Thumbnails {
    id: WidgetId,
    label: String,
    tiles: Vec<(Option<Image>, Text)>,
    selected: Option<usize>,
    on_select: Box<dyn FnMut(usize)>,
    columns: usize,
    row_height: f32,
    frame: Rect,
}

impl Thumbnails {
    pub fn new(
        label: &str,
        tiles: Vec<Thumbnail>,
        selected: Option<usize>,
        on_select: impl FnMut(usize) + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            tiles: tiles
                .into_iter()
                .map(|tile| (tile.image, Text::new(&tile.label, Tone::Muted).caption()))
                .collect(),
            selected,
            on_select: Box::new(on_select),
            columns: 1,
            row_height: THUMBNAIL.height,
            frame: Rect::default(),
        }
    }

    fn select(&mut self, index: usize) {
        if index < self.tiles.len() && self.selected != Some(index) {
            self.selected = Some(index);
            (self.on_select)(index);
        }
    }

    fn tile(&self, index: usize) -> Rect {
        let column = index % self.columns;
        let row = index / self.columns;
        Rect::new(
            self.frame.x + ROW_PADDING + column as f32 * (THUMBNAIL.width + THUMBNAIL_GAP),
            self.frame.y + ROW_PADDING + row as f32 * (self.row_height + THUMBNAIL_GAP),
            THUMBNAIL.width,
            THUMBNAIL.height,
        )
    }

    fn step(&mut self, by: isize) {
        let current = self.selected.unwrap_or(0);
        if let Some(next) = current.checked_add_signed(by) {
            self.select(next);
        }
    }
}

impl Widget for Thumbnails {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let label = self
            .tiles
            .iter_mut()
            .map(|(_, text)| text.measure(cx).height)
            .fold(0.0, f32::max);
        self.row_height = THUMBNAIL.height + THUMBNAIL_LABEL_GAP + label;
        let room = available.width - 2.0 * ROW_PADDING + THUMBNAIL_GAP;
        self.columns = ((room / (THUMBNAIL.width + THUMBNAIL_GAP)).floor() as usize).max(1);
        let rows = self.tiles.len().div_ceil(self.columns).max(1);
        Size::new(
            available.width,
            2.0 * ROW_PADDING + rows as f32 * self.row_height + (rows - 1) as f32 * THUMBNAIL_GAP,
        )
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for (index, (image, label)) in self.tiles.iter().enumerate() {
            let rect = self.tile(index);
            let shape = Shape::rounded(rect, THUMBNAIL_RADIUS);
            let picture = match image {
                Some(image) => Item::Image {
                    rect,
                    image: image.clone(),
                },
                None => Item::Fill {
                    shape,
                    paint: cx.palette.well.into(),
                },
            };
            items.push(Item::Clip {
                shape,
                items: vec![picture],
            });
            items.push(Item::Stroke {
                shape: Shape::rounded(rect.inset(0.5), THUMBNAIL_RADIUS - 0.5),
                width: 1.0,
                paint: cx.palette.hairline.into(),
            });
            if self.selected == Some(index) {
                items.push(Item::Stroke {
                    shape: Shape::rounded(rect.inset(-3.0), THUMBNAIL_RADIUS + 3.0),
                    width: 3.0,
                    paint: cx.palette.accent.into(),
                });
                if cx.is_focused(self.id) {
                    focus_ring(
                        cx,
                        Shape::rounded(rect.inset(-6.0), THUMBNAIL_RADIUS + 6.0),
                        items,
                    );
                }
            }
            let size = label.size();
            label.paint(
                cx,
                Point::new(
                    rect.x + (rect.width - size.width) / 2.0,
                    rect.y + rect.height + THUMBNAIL_LABEL_GAP,
                ),
                items,
            );
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::RadioGroup);
        node.set_label(self.label.clone());
        if let Some((_, label)) = self.selected.and_then(|index| self.tiles.get(index)) {
            node.set_value(label.as_str().to_owned());
        }
        node.add_action(Action::Increment);
        node.add_action(Action::Decrement);
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        let columns = self.columns as isize;
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => {
                match (0..self.tiles.len())
                    .find(|index| contains(self.tile(*index), pointer.position))
                {
                    Some(index) => self.select(index),
                    None => return false,
                }
            }
            Event::Key { key: Key::Left, .. } => self.step(-1),
            Event::Key {
                key: Key::Right, ..
            } => self.step(1),
            Event::Key { key: Key::Up, .. } => self.step(-columns),
            Event::Key { key: Key::Down, .. } => self.step(columns),
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        match request.action {
            Action::Increment => self.step(1),
            Action::Decrement => self.step(-1),
            _ => return false,
        }
        true
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation
    }
}

const SWATCH_SIZE: f32 = 22.0;
const SWATCH_GAP: f32 = 12.0;
const SWATCH_RING: f32 = 2.0;
const SWATCH_RING_GAP: f32 = 2.0;

pub struct Swatch {
    pub label: String,
    pub light: Color,
    pub dark: Color,
}

pub struct Swatches {
    id: WidgetId,
    label: String,
    swatches: Vec<Swatch>,
    selected: Option<usize>,
    on_select: Box<dyn FnMut(usize)>,
    frame: Rect,
}

impl Swatches {
    pub fn new(
        label: &str,
        swatches: Vec<Swatch>,
        selected: Option<usize>,
        on_select: impl FnMut(usize) + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            swatches,
            selected,
            on_select: Box::new(on_select),
            frame: Rect::default(),
        }
    }

    fn select(&mut self, index: usize) {
        if index < self.swatches.len() && self.selected != Some(index) {
            self.selected = Some(index);
            (self.on_select)(index);
        }
    }

    fn step(&mut self, by: isize) {
        if let Some(next) = self.selected.unwrap_or(0).checked_add_signed(by) {
            self.select(next);
        }
    }

    fn swatch(&self, index: usize) -> Rect {
        let margin = SWATCH_RING + SWATCH_RING_GAP;
        Rect::new(
            self.frame.x + margin + index as f32 * (SWATCH_SIZE + SWATCH_GAP),
            self.frame.y + (self.frame.height - SWATCH_SIZE) / 2.0,
            SWATCH_SIZE,
            SWATCH_SIZE,
        )
    }
}

impl Widget for Swatches {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, _available: Size) -> Size {
        let count = self.swatches.len() as f32;
        let margin = 2.0 * (SWATCH_RING + SWATCH_RING_GAP);
        Size::new(
            count * SWATCH_SIZE + (count - 1.0).max(0.0) * SWATCH_GAP + margin,
            SWATCH_SIZE + margin,
        )
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for (index, swatch) in self.swatches.iter().enumerate() {
            let rect = self.swatch(index);
            let color = match cx.palette.scheme {
                Scheme::Light => swatch.light,
                Scheme::Dark => swatch.dark,
            };
            items.push(Item::Fill {
                shape: Shape::rounded(rect, SWATCH_SIZE / 2.0),
                paint: color.into(),
            });
            items.push(Item::Stroke {
                shape: Shape::rounded(rect.inset(0.5), SWATCH_SIZE / 2.0 - 0.5),
                width: 1.0,
                paint: cx.palette.ink.with_alpha(28).into(),
            });
            if self.selected == Some(index) {
                let ring = rect.inset(-(SWATCH_RING_GAP + SWATCH_RING / 2.0));
                items.push(Item::Stroke {
                    shape: Shape::rounded(ring, ring.width / 2.0),
                    width: SWATCH_RING,
                    paint: color.into(),
                });
                if cx.is_focused(self.id) {
                    let outer = rect.inset(-(SWATCH_RING_GAP + SWATCH_RING));
                    focus_ring(cx, Shape::rounded(outer, outer.width / 2.0), items);
                }
            }
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::RadioGroup);
        node.set_label(self.label.clone());
        if let Some(swatch) = self.selected.and_then(|index| self.swatches.get(index)) {
            node.set_value(swatch.label.clone());
        }
        node.add_action(Action::Increment);
        node.add_action(Action::Decrement);
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => {
                match (0..self.swatches.len()).find(|index| {
                    contains(
                        self.swatch(*index).inset(-SWATCH_GAP / 2.0),
                        pointer.position,
                    )
                }) {
                    Some(index) => self.select(index),
                    None => return false,
                }
            }
            Event::Key {
                key: Key::Left | Key::Up,
                ..
            } => self.step(-1),
            Event::Key {
                key: Key::Right | Key::Down,
                ..
            } => self.step(1),
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        match request.action {
            Action::Increment => self.step(1),
            Action::Decrement => self.step(-1),
            _ => return false,
        }
        true
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation
    }
}

const USAGE_BAR_HEIGHT: f32 = 12.0;
const USAGE_BAR_RADIUS: f32 = Radius::XS;
const SWATCH: f32 = 8.0;
const SEGMENT_SHARES: [f32; 4] = [1.0, 0.7, 0.45, 0.25];

pub struct Segment {
    pub label: String,
    pub bytes: u64,
}

pub struct UsageBar {
    id: WidgetId,
    summary: String,
    segments: Vec<Segment>,
    capacity: u64,
    legend: Vec<(Text, Point)>,
    frame: Rect,
}

impl UsageBar {
    pub fn new(summary: &str, segments: Vec<Segment>, capacity: u64) -> Self {
        Self {
            id: WidgetId::next(),
            summary: summary.to_owned(),
            segments,
            capacity,
            legend: Vec::new(),
            frame: Rect::default(),
        }
    }

    fn color(cx: &Context, index: usize) -> Color {
        let share = SEGMENT_SHARES[index % SEGMENT_SHARES.len()];
        cx.palette.well.mix(cx.palette.accent, share)
    }
}

impl Widget for UsageBar {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let width = (available.width - 2.0 * ROW_PADDING).max(0.0);
        let mut legend = Vec::new();
        let (mut x, mut y, mut line) = (0.0_f32, 0.0_f32, 0.0_f32);
        for segment in &self.segments {
            let mut text = Text::new(&segment.label, Tone::Primary).caption();
            let size = text.measure(cx);
            let item = SWATCH + Spacing::XS as f32 + size.width;
            if x > 0.0 && x + item > width {
                x = 0.0;
                y += line + Spacing::XS as f32;
                line = 0.0;
            }
            legend.push((text, Point::new(x, y)));
            x += item + Spacing::L as f32;
            line = line.max(size.height);
        }
        self.legend = legend;
        let legend_height = if self.segments.is_empty() {
            0.0
        } else {
            y + line + Spacing::S as f32
        };
        Size::new(
            available.width,
            (2.0 * Spacing::M as f32 + USAGE_BAR_HEIGHT + legend_height).ceil(),
        )
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let track = Rect::new(
            self.frame.x + ROW_PADDING,
            self.frame.y + Spacing::M as f32,
            (self.frame.width - 2.0 * ROW_PADDING).max(0.0),
            USAGE_BAR_HEIGHT,
        );
        let mut fills = vec![Item::Fill {
            shape: Shape::rect(track),
            paint: cx.palette.well.into(),
        }];
        let mut x = track.x;
        for (index, segment) in self.segments.iter().enumerate() {
            let share = segment.bytes as f64 / self.capacity.max(1) as f64;
            let width = (track.width * share as f32).max(1.0);
            fills.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    x.round(),
                    track.y,
                    (x + width).round() - x.round(),
                    track.height,
                )),
                paint: Self::color(cx, index).into(),
            });
            x += width;
        }
        items.push(Item::Clip {
            shape: Shape::rounded(track, USAGE_BAR_RADIUS),
            items: fills,
        });
        let origin = Point::new(track.x, track.y + track.height + Spacing::S as f32);
        for (index, (text, at)) in self.legend.iter().enumerate() {
            let y = origin.y + at.y;
            let swatch_y = y + ((text.size().height - SWATCH) / 2.0).round();
            items.push(Item::Fill {
                shape: Shape::rounded(Rect::new(origin.x + at.x, swatch_y, SWATCH, SWATCH), 2.0),
                paint: Self::color(cx, index).into(),
            });
            text.paint(
                cx,
                Point::new(origin.x + at.x + SWATCH + Spacing::XS as f32, y),
                items,
            );
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Meter);
        let used: u64 = self.segments.iter().map(|segment| segment.bytes).sum();
        let legend: Vec<&str> = self
            .segments
            .iter()
            .map(|segment| segment.label.as_str())
            .collect();
        node.set_label(format!("{}: {}", self.summary, legend.join(", ")));
        node.set_numeric_value(used as f64);
        node.set_min_numeric_value(0.0);
        node.set_max_numeric_value(self.capacity as f64);
        node
    }
}

const HISTORY_HEIGHT: f32 = 72.0;
const HISTORY_BUCKETS: u32 = 48;
const HISTORY_GAP: f32 = 2.0;
const HISTORY_SPAN: u32 = 24 * 3600;

pub fn bucket_levels(points: &[(u32, f64)], buckets: u32, span: u32) -> Vec<Option<f64>> {
    let Some(end) = points.iter().map(|(time, _)| *time).max() else {
        return vec![None; buckets as usize];
    };
    let start = end.saturating_sub(span);
    (1..=buckets)
        .map(|bucket| {
            let until = start + span / buckets * bucket;
            points
                .iter()
                .filter(|(time, _)| *time <= until)
                .max_by_key(|(time, _)| *time)
                .map(|(_, level)| level.clamp(0.0, 100.0))
        })
        .collect()
}

pub struct LevelHistory {
    id: WidgetId,
    label: String,
    levels: Vec<Option<f64>>,
    captions: Vec<Text>,
    frame: Rect,
}

impl LevelHistory {
    pub fn new(label: &str, points: Vec<(u32, f64)>, captions: [String; 3]) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            levels: bucket_levels(&points, HISTORY_BUCKETS, HISTORY_SPAN),
            captions: captions
                .iter()
                .map(|caption| Text::new(caption, Tone::Muted).caption())
                .collect(),
            frame: Rect::default(),
        }
    }
}

impl Widget for LevelHistory {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let captions = self
            .captions
            .iter_mut()
            .map(|caption| caption.measure(cx).height)
            .fold(0.0, f32::max);
        Size::new(
            available.width,
            (2.0 * Spacing::M as f32 + HISTORY_HEIGHT + Spacing::XS as f32 + captions).ceil(),
        )
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let chart = Rect::new(
            self.frame.x + ROW_PADDING,
            self.frame.y + Spacing::M as f32,
            (self.frame.width - 2.0 * ROW_PADDING).max(0.0),
            HISTORY_HEIGHT,
        );
        for fraction in [0.0, 0.5, 1.0] {
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    chart.x,
                    (chart.y + chart.height * fraction).round(),
                    chart.width,
                    1.0,
                )),
                paint: cx.palette.hairline.into(),
            });
        }
        let count = self.levels.len().max(1) as f32;
        let width = (chart.width - HISTORY_GAP * (count - 1.0)) / count;
        for (index, level) in self.levels.iter().enumerate() {
            let Some(level) = level else { continue };
            let height = (chart.height * (*level as f32) / 100.0).max(1.0);
            let x = chart.x + index as f32 * (width + HISTORY_GAP);
            items.push(Item::Fill {
                shape: Shape::rounded(
                    Rect::new(x, chart.y + chart.height - height, width, height),
                    1.0,
                ),
                paint: cx.palette.accent.into(),
            });
        }
        let y = chart.y + chart.height + Spacing::XS as f32;
        let last = self.captions.len().saturating_sub(1).max(1) as f32;
        for (index, caption) in self.captions.iter().enumerate() {
            let anchor = chart.x + chart.width * index as f32 / last;
            let x = anchor - caption.size().width * index as f32 / last;
            caption.paint(cx, Point::new(x, y), items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Image);
        let known: Vec<f64> = self.levels.iter().flatten().copied().collect();
        let description = match (known.first(), known.last()) {
            (Some(first), Some(last)) => format!("{}: {first:.0}% → {last:.0}%", self.label),
            _ => self.label.clone(),
        };
        node.set_label(description);
        node
    }
}

#[derive(Default)]
pub struct Crossfade {
    progress: Option<Animated>,
    before: Vec<Item>,
}

impl Crossfade {
    pub fn start(&mut self, now: Instant) {
        let mut progress = Animated::new(0.0);
        progress.animate_at(now, false, 1.0, Spring::FADE);
        self.progress = Some(progress);
    }

    pub fn stop(&mut self) {
        self.progress = None;
    }
}

pub struct Fade {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    crossfade: Rc<RefCell<Crossfade>>,
    frame: Rect,
}

impl Fade {
    pub fn new(child: Box<dyn Widget>, crossfade: Rc<RefCell<Crossfade>>) -> Self {
        Self {
            id: WidgetId::next(),
            children: vec![child],
            crossfade,
            frame: Rect::default(),
        }
    }
}

impl Widget for Fade {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.children
            .iter_mut()
            .map(|child| child.measure(cx, available))
            .next()
            .unwrap_or_default()
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        for child in &mut self.children {
            child.place(cx, frame);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let mut after = Vec::new();
        for child in &self.children {
            child.paint(cx, &mut after);
        }
        let mut crossfade = self.crossfade.borrow_mut();
        let alpha = crossfade
            .progress
            .map_or(1.0, |progress| progress.value(cx).clamp(0.0, 1.0));
        if alpha >= 1.0 {
            crossfade.progress = None;
            crossfade.before.clone_from(&after);
            items.extend(after);
            return;
        }
        items.push(Item::Opacity {
            alpha: 1.0 - alpha,
            items: crossfade.before.clone(),
        });
        items.push(Item::Opacity {
            alpha,
            items: after,
        });
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::GenericContainer)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}
