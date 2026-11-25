use std::cell::Cell;
use std::rc::Rc;

use accesskit::{Node, Role};
use bande_graphics::{Item, Rect};

use super::{Context, Size, Widget, WidgetId};

/// A sheet the app closes itself by setting `closed`, such as one that shows work under way.
pub struct Closable {
    id: WidgetId,
    child: [Box<dyn Widget>; 1],
    closed: Rc<Cell<bool>>,
}

impl Closable {
    pub fn new(child: impl Widget + 'static, closed: Rc<Cell<bool>>) -> Self {
        Self {
            id: WidgetId::next(),
            child: [Box::new(child)],
            closed,
        }
    }
}

impl Widget for Closable {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.child[0].measure(cx, available)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.child[0].place(cx, frame);
    }

    fn frame(&self) -> Rect {
        self.child[0].frame()
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        self.child[0].paint(cx, items);
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::GenericContainer)
    }

    fn dismissed(&self) -> bool {
        self.closed.get() || self.child[0].dismissed()
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.child
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.child
    }
}
