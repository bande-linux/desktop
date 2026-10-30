use accesskit::{Node, Role};
use unbound_toolkit::ui::{Context, Size, Widget, WidgetId};
use unbound_toolkit::{Item, Rect};

pub(crate) struct Inset {
    id: WidgetId,
    top: f32,
    child: [Box<dyn Widget>; 1],
    frame: Rect,
}

impl Inset {
    pub(crate) fn new(top: f32, child: impl Widget + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            top,
            child: [Box::new(child)],
            frame: Rect::default(),
        }
    }
}

impl Widget for Inset {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let size = self.child[0].measure(cx, available);
        Size::new(size.width, size.height + self.top)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        self.child[0].place(
            cx,
            Rect::new(
                frame.x,
                frame.y + self.top,
                frame.width,
                frame.height - self.top,
            ),
        );
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        self.child[0].paint(cx, items);
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::Unknown)
    }

    fn dismissed(&self) -> bool {
        self.child[0].dismissed()
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.child
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.child
    }
}

pub(crate) struct Anchor {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    at: Rect,
    frame: Rect,
}

impl Anchor {
    pub(crate) fn new() -> Self {
        Self {
            id: WidgetId::next(),
            children: Vec::new(),
            at: Rect::default(),
            frame: Rect::default(),
        }
    }

    pub(crate) fn hold(&mut self, child: Option<Box<dyn Widget>>, at: Rect) {
        self.children = child.into_iter().collect();
        self.at = at;
    }

    pub(crate) fn move_to(&mut self, at: Rect) {
        self.at = at;
    }
}

impl Widget for Anchor {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        for child in &mut self.children {
            child.measure(cx, Size::new(self.at.width, self.at.height));
            child.place(cx, self.at);
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
        Node::new(Role::Unknown)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}
