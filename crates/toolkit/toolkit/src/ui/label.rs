use accesskit::{Node, Role};

use super::{Context, Size, Widget, WidgetId};
use crate::scene::{Item, Point, Rect};
use crate::text::{Paragraph, TextAlign, TextStyle};

pub struct Label {
    id: WidgetId,
    text: String,
    size: f32,
    weight: f32,
    paragraph: Option<Paragraph>,
    frame: Rect,
}

impl Label {
    pub fn new(text: &str) -> Self {
        Self {
            id: WidgetId::next(),
            text: text.to_owned(),
            size: 13.0,
            weight: TextStyle::REGULAR,
            paragraph: None,
            frame: Rect::default(),
        }
    }

    pub fn heading(text: &str) -> Self {
        Self {
            size: 22.0,
            weight: TextStyle::SEMIBOLD,
            ..Self::new(text)
        }
    }
}

impl Widget for Label {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let style = TextStyle::new(self.size, cx.palette.label_primary).weight(self.weight);
        let paragraph =
            cx.text
                .paragraph(&self.text, style, Some(available.width), TextAlign::Start);
        let size = Size::new(paragraph.width().ceil(), paragraph.height().ceil());
        self.paragraph = Some(paragraph);
        size
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, _cx: &Context, items: &mut Vec<Item>) {
        if let Some(paragraph) = &self.paragraph {
            items.push(Item::Text {
                origin: Point::new(self.frame.x, self.frame.y),
                paragraph: paragraph.clone(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Label);
        node.set_value(self.text.clone());
        node
    }
}
