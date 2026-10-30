use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, Node, Role};
use unbound_toolkit::ui::{Context, Event, Key, Size, Widget, WidgetId};
use unbound_toolkit::{
    Color, Image, Item, Paragraph, Point, Rect, Shape, Spacing, TextAlign, TextStyle,
};

pub const SIDEBAR_WIDTH: f32 = 220.0;
const MARGIN: f32 = Spacing::M as f32;
const BODY_SIZE: f32 = 13.0;
const CAPTION_SIZE: f32 = 11.0;
const SIDEBAR_ROW_HEIGHT: f32 = 28.0;
const ROW_PADDING: f32 = Spacing::M as f32;
const ROW_MIN_HEIGHT: f32 = 40.0;
const CONTROL_WIDTH: f32 = 320.0;
const CARD_RADIUS: f32 = 10.0;
const FOCUS_RING: f32 = 3.0;
const HIGHLIGHT_ALPHA: u8 = 48;
const SCROLL_BAR_WIDTH: f32 = 4.0;
const UNBOUNDED: f32 = 1.0e6;
const WHITE: Color = Color::hex(0xFFFFFF);

fn contains(frame: Rect, point: Point) -> bool {
    point.x >= frame.x
        && point.y >= frame.y
        && point.x < frame.x + frame.width
        && point.y < frame.y + frame.height
}

fn focus_ring(cx: &Context, shape: Shape, items: &mut Vec<Item>) {
    items.push(Item::Stroke {
        shape: Shape::rounded(
            shape.rect.inset(-FOCUS_RING / 2.0),
            shape.radius + FOCUS_RING / 2.0,
        ),
        width: FOCUS_RING,
        paint: cx.palette.accent_color.with_alpha(128).into(),
    });
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
    OnAccent,
}

pub struct Text {
    text: String,
    highlights: Vec<Range<usize>>,
    size: f32,
    weight: f32,
    tone: Tone,
    runs: Vec<(f32, Paragraph, bool)>,
    measured: Size,
}

impl Text {
    pub fn new(text: &str, tone: Tone) -> Self {
        Self {
            text: text.to_owned(),
            highlights: Vec::new(),
            size: BODY_SIZE,
            weight: TextStyle::REGULAR,
            tone,
            runs: Vec::new(),
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

    fn segments(&self) -> Vec<(&str, bool)> {
        let mut segments = Vec::new();
        let mut at = 0;
        for range in &self.highlights {
            if range.start > at {
                segments.push((&self.text[at..range.start], false));
            }
            segments.push((&self.text[range.clone()], true));
            at = range.end;
        }
        if at < self.text.len() || segments.is_empty() {
            segments.push((&self.text[at..], false));
        }
        segments
    }

    pub fn measure(&mut self, cx: &mut Context) -> Size {
        let color = match self.tone {
            Tone::Primary => cx.palette.label_primary,
            Tone::Muted => cx.palette.label_muted,
            Tone::OnAccent => WHITE,
        };
        let mut x = 0.0;
        let mut height: f32 = 0.0;
        let mut runs = Vec::new();
        for (segment, highlighted) in self.segments() {
            let weight = if highlighted {
                TextStyle::SEMIBOLD
            } else {
                self.weight
            };
            let paragraph = cx.text.paragraph(
                segment,
                TextStyle::new(self.size, color).weight(weight),
                None,
                TextAlign::Start,
            );
            let width = paragraph.width();
            height = height.max(paragraph.height());
            runs.push((x, paragraph, highlighted));
            x += width;
        }
        self.runs = runs;
        self.measured = Size::new(x.ceil(), height.ceil());
        self.measured
    }

    pub fn size(&self) -> Size {
        self.measured
    }

    pub fn paint(&self, cx: &Context, origin: Point, items: &mut Vec<Item>) {
        for (x, paragraph, highlighted) in &self.runs {
            let at = Point::new((origin.x + x).round(), origin.y.round());
            if *highlighted {
                items.push(Item::Fill {
                    shape: Shape::rounded(
                        Rect::new(
                            at.x - 1.0,
                            at.y,
                            paragraph.width() + 2.0,
                            paragraph.height(),
                        ),
                        3.0,
                    ),
                    paint: cx.palette.accent_color.with_alpha(HIGHLIGHT_ALPHA).into(),
                });
            }
            items.push(Item::Text {
                origin: at,
                paragraph: paragraph.clone(),
            });
        }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }
}

pub struct Root {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    sizes: Vec<Size>,
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
            frame: Rect::default(),
        }
    }
}

impl Widget for Root {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let sidebar = Size::new(SIDEBAR_WIDTH - 2.0 * MARGIN, available.height);
        let content = Size::new((available.width - SIDEBAR_WIDTH).max(0.0), available.height);
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
        let width = SIDEBAR_WIDTH - 2.0 * MARGIN;
        let frames = [
            Rect::new(MARGIN, MARGIN, width, search_height),
            Rect::new(
                MARGIN,
                2.0 * MARGIN + search_height,
                width,
                (frame.height - 3.0 * MARGIN - search_height).max(0.0),
            ),
            Rect::new(
                SIDEBAR_WIDTH,
                0.0,
                (frame.width - SIDEBAR_WIDTH).max(0.0),
                frame.height,
            ),
        ];
        for (child, child_frame) in self.children.iter_mut().zip(frames) {
            child.place(cx, child_frame);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(0.0, 0.0, SIDEBAR_WIDTH, self.frame.height)),
            paint: cx.palette.surface_elevated.into(),
        });
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(SIDEBAR_WIDTH - 1.0, 0.0, 1.0, self.frame.height)),
            paint: cx.palette.separator_color.into(),
        });
        for child in &self.children {
            child.paint(cx, items);
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
    frame: Rect,
}

impl Caption {
    pub fn new(text: Text) -> Self {
        Self {
            id: WidgetId::next(),
            text,
            frame: Rect::default(),
        }
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
        self.text
            .paint(cx, Point::new(self.frame.x, self.frame.y), items);
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Label);
        node.set_value(self.text.as_str().to_owned());
        node
    }
}

pub struct SidebarRow {
    id: WidgetId,
    text: Text,
    selected: bool,
    on_select: Box<dyn FnMut()>,
    on_step: Box<dyn FnMut(isize)>,
    frame: Rect,
}

impl SidebarRow {
    pub fn new(
        text: Text,
        selected: bool,
        on_select: impl FnMut() + 'static,
        on_step: impl FnMut(isize) + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            text,
            selected,
            on_select: Box::new(on_select),
            on_step: Box::new(on_step),
            frame: Rect::default(),
        }
    }
}

impl Widget for SidebarRow {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.text.measure(cx);
        Size::new(available.width, SIDEBAR_ROW_HEIGHT)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let shape = Shape::rounded(self.frame, 6.0);
        if self.selected {
            items.push(Item::Fill {
                shape,
                paint: cx.palette.accent_color.into(),
            });
        }
        if cx.is_focused(self.id) {
            focus_ring(cx, shape, items);
        }
        let height = self.text.size().height;
        self.text.paint(
            cx,
            Point::new(
                self.frame.x + 10.0,
                self.frame.y + (self.frame.height - height) / 2.0,
            ),
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
                paint: cx.palette.label_muted.with_alpha(110).into(),
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
        items.push(Item::Fill {
            shape: Shape::rounded(self.frame, CARD_RADIUS),
            paint: cx.palette.surface_elevated.into(),
        });
        items.push(Item::Stroke {
            shape: Shape::rounded(self.frame.inset(0.5), CARD_RADIUS - 0.5),
            width: 1.0,
            paint: cx.palette.separator_color.with_alpha(160).into(),
        });
        for (index, child) in self.children.iter().enumerate() {
            if index > 0 {
                let frame = child.frame();
                items.push(Item::Fill {
                    shape: Shape::rect(Rect::new(
                        frame.x + ROW_PADDING,
                        frame.y,
                        frame.width - 2.0 * ROW_PADDING,
                        1.0,
                    )),
                    paint: cx.palette.separator_color.into(),
                });
            }
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
        self.label.measure(cx);
        if let Some(detail) = &mut self.detail {
            detail.measure(cx);
        }
        let control_width = CONTROL_WIDTH.min(available.width / 2.0);
        self.control = self
            .children
            .iter_mut()
            .map(|child| child.measure(cx, Size::new(control_width, UNBOUNDED)))
            .next()
            .unwrap_or_default();
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
        let shape = Shape::rounded(self.frame.inset(2.0), CARD_RADIUS - 2.0);
        if self.highlighted {
            items.push(Item::Stroke {
                shape,
                width: 2.0,
                paint: cx.palette.accent_color.into(),
            });
        }
        if self.pressed {
            items.push(Item::Fill {
                shape,
                paint: cx.palette.separator_color.with_alpha(120).into(),
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
                paint: cx.palette.label_muted.into(),
            });
            items.push(Item::Fill {
                shape: Shape::rounded(body, 1.5),
                paint: cx.palette.label_muted.into(),
            });
        }
        x += LOCK_WIDTH + BAR_GAP;
        for bar in 0..BARS {
            let height = SIGNAL_HEIGHT * f32::from(bar + 1) / f32::from(BARS);
            let color = if bar < self.bars() {
                cx.palette.label_primary
            } else {
                cx.palette.separator_color
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
                    paint: cx.palette.accent_color.into(),
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
const THUMBNAIL_RADIUS: f32 = 8.0;
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
                    paint: cx.palette.label_muted.with_alpha(36).into(),
                },
            };
            items.push(Item::Clip {
                shape,
                items: vec![picture],
            });
            items.push(Item::Stroke {
                shape: Shape::rounded(rect.inset(0.5), THUMBNAIL_RADIUS - 0.5),
                width: 1.0,
                paint: cx.palette.separator_color.into(),
            });
            if self.selected == Some(index) {
                items.push(Item::Stroke {
                    shape: Shape::rounded(rect.inset(-3.0), THUMBNAIL_RADIUS + 3.0),
                    width: 3.0,
                    paint: cx.palette.accent_color.into(),
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

const USAGE_BAR_HEIGHT: f32 = 12.0;
const USAGE_BAR_RADIUS: f32 = 4.0;
const SWATCH: f32 = 8.0;
const SEGMENT_ALPHAS: [u8; 4] = [255, 185, 120, 70];

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
        let alpha = SEGMENT_ALPHAS[index % SEGMENT_ALPHAS.len()];
        cx.palette.accent_color.with_alpha(alpha)
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
            paint: cx.palette.separator_color.into(),
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
