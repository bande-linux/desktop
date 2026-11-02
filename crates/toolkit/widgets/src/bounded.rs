use accesskit::{Node, Role};
use bande_graphics::{Item, Rect};

use super::{Context, Size, Widget, WidgetId};

pub struct Bounded {
    id: WidgetId,
    height: f32,
    children: Vec<Box<dyn Widget>>,
    frame: Rect,
}

impl Bounded {
    pub fn new(child: impl Widget + 'static, height: f32) -> Self {
        Self {
            id: WidgetId::next(),
            height,
            children: vec![Box::new(child)],
            frame: Rect::default(),
        }
    }
}

impl Widget for Bounded {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let size = Size::new(available.width, self.height.min(available.height));
        for child in &mut self.children {
            child.measure(cx, size);
        }
        size
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
