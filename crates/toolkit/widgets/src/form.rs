use accesskit::{Node, Role};

use super::{Context, Size, Widget, WidgetId};
use bande_graphics::{Item, Paragraph, Point, Rect, TextAlign, TextRole, TextStyle};

const MARGIN: f32 = 24.0;
const ROW: f32 = 44.0;
const HEADING: f32 = 32.0;
const ROW_PADDING: f32 = 8.0;
const LABEL_SHARE: f32 = 0.55;

pub enum FormLine {
    Heading(String),
    Row(String, Box<dyn Widget>),
    Wide(Box<dyn Widget>),
}

enum Line {
    Heading(String),
    Row(String),
    Wide,
}

pub struct Form {
    id: WidgetId,
    label: String,
    lines: Vec<Line>,
    children: Vec<Box<dyn Widget>>,
    texts: Vec<(Point, Paragraph)>,
    heights: Vec<f32>,
    frame: Rect,
}

impl Form {
    pub fn new(label: &str, lines: Vec<FormLine>) -> Self {
        let mut children = Vec::new();
        let lines = lines
            .into_iter()
            .map(|line| match line {
                FormLine::Heading(text) => Line::Heading(text),
                FormLine::Row(text, control) => {
                    children.push(control);
                    Line::Row(text)
                }
                FormLine::Wide(widget) => {
                    children.push(widget);
                    Line::Wide
                }
            })
            .collect();
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            lines,
            children,
            texts: Vec::new(),
            heights: Vec::new(),
            frame: Rect::default(),
        }
    }
}

fn row_label(cx: &mut Context, text: &str, width: f32) -> Paragraph {
    cx.text.paragraph(
        text,
        TextStyle::role(TextRole::Body, cx.palette.ink),
        Some(width * LABEL_SHARE),
        TextAlign::Start,
    )
}

impl Widget for Form {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let grown = cx.grown_line();
        let width = available.width - 2.0 * MARGIN;
        let mut children = self.children.iter_mut();
        let mut heights = Vec::with_capacity(self.lines.len());
        for line in &self.lines {
            heights.push(match line {
                Line::Heading(_) => HEADING + grown,
                Line::Row(text) => {
                    if let Some(child) = children.next() {
                        child.measure(cx, available);
                    }
                    let label = row_label(cx, text, width);
                    (ROW + grown).max((label.height() + 2.0 * ROW_PADDING).ceil())
                }
                Line::Wide => children
                    .next()
                    .map_or(0.0, |child| child.measure(cx, Size::new(width, ROW)).height),
            });
        }
        self.heights = heights;
        Size::new(available.width, self.heights.iter().sum::<f32>() + MARGIN)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        self.texts.clear();
        let width = frame.width - 2.0 * MARGIN;
        let mut y = frame.y + MARGIN / 2.0;
        let mut children = self.children.iter_mut();
        for (line, &height) in self.lines.iter().zip(&self.heights) {
            match line {
                Line::Heading(text) => {
                    let paragraph = cx.text.paragraph(
                        text,
                        TextStyle::role(TextRole::Label, cx.palette.ink3),
                        None,
                        TextAlign::Start,
                    );
                    let origin = Point::new(
                        frame.x + MARGIN,
                        (y + height - paragraph.height() - 6.0).round(),
                    );
                    self.texts.push((origin, paragraph));
                }
                Line::Row(text) => {
                    let paragraph = row_label(cx, text, width);
                    let origin = Point::new(
                        frame.x + MARGIN,
                        (y + (height - paragraph.height()) / 2.0).round(),
                    );
                    self.texts.push((origin, paragraph));
                    if let Some(child) = children.next() {
                        let size =
                            child.measure(cx, Size::new(width * (1.0 - LABEL_SHARE), height));
                        child.place(
                            cx,
                            Rect::new(
                                frame.x + frame.width - MARGIN - size.width,
                                (y + (height - size.height) / 2.0).round(),
                                size.width,
                                size.height,
                            ),
                        );
                    }
                }
                Line::Wide => {
                    if let Some(child) = children.next() {
                        child.place(cx, Rect::new(frame.x + MARGIN, y, width, height));
                    }
                }
            }
            y += height;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for (origin, paragraph) in &self.texts {
            items.push(Item::Text {
                origin: *origin,
                paragraph: paragraph.clone(),
            });
        }
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Group);
        node.set_label(self.label.clone());
        node
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}
