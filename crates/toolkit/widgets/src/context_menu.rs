use accesskit::{Node, Role};

use super::{Context, Menu, Size, Widget, WidgetId};
use unbound_graphics::{Item, Point, Rect};

pub struct ContextMenu {
    id: WidgetId,
    child: [Box<dyn Widget>; 1],
    menu: Box<dyn FnMut() -> Menu>,
    on_action: Box<dyn FnMut(&str)>,
}

impl ContextMenu {
    pub fn new(
        child: impl Widget + 'static,
        menu: impl FnMut() -> Menu + 'static,
        on_action: impl FnMut(&str) + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            child: [Box::new(child)],
            menu: Box::new(menu),
            on_action: Box::new(on_action),
        }
    }
}

impl Widget for ContextMenu {
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

    fn context_menu(&mut self, _cx: &mut Context, _point: Point) -> Option<Menu> {
        Some((self.menu)())
    }

    fn menu_action(&mut self, _cx: &mut Context, action: &str) -> bool {
        (self.on_action)(action);
        true
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.child
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.child
    }
}
