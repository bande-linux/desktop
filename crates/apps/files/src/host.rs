use std::cell::Cell;
use std::rc::Rc;

use accesskit::{Node, Role};
use unbound_toolkit::ui::{Context, Size, Widget, WidgetId};
use unbound_toolkit::{Item, Rect, Shape};

const FIELD_RADIUS: f32 = 4.0;

pub(crate) struct Host {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    frames: Vec<Rect>,
    framed: Vec<bool>,
    frame: Rect,
    painted: Rc<Cell<usize>>,
}

impl Host {
    pub(crate) fn new(painted: Rc<Cell<usize>>) -> Self {
        Self {
            id: WidgetId::next(),
            children: Vec::new(),
            frames: Vec::new(),
            framed: Vec::new(),
            frame: Rect::default(),
            painted,
        }
    }

    pub(crate) fn insert(&mut self, widget: Box<dyn Widget>, frame: Rect, framed: bool) {
        self.children.push(widget);
        self.frames.push(frame);
        self.framed.push(framed);
    }

    fn index(&self, id: WidgetId) -> Option<usize> {
        self.children.iter().position(|child| child.id() == id)
    }

    pub(crate) fn remove(&mut self, id: WidgetId) {
        if let Some(index) = self.index(id) {
            self.children.remove(index);
            self.frames.remove(index);
            self.framed.remove(index);
        }
    }

    pub(crate) fn set_frame(&mut self, id: WidgetId, frame: Rect) {
        if let Some(index) = self.index(id) {
            self.frames[index] = frame;
        }
    }

    pub(crate) fn frame_of(&self, id: WidgetId) -> Option<Rect> {
        self.index(id).map(|index| self.frames[index])
    }
}

fn shown(frame: Rect) -> bool {
    frame.width > 0.0 && frame.height > 0.0
}

impl Widget for Host {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        for (child, frame) in self.children.iter_mut().zip(&self.frames) {
            child.measure(cx, Size::new(frame.width, frame.height));
            child.place(cx, *frame);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for ((child, frame), framed) in self.children.iter().zip(&self.frames).zip(&self.framed) {
            if !shown(*frame) {
                continue;
            }
            if *framed {
                let shape = Shape::rounded(child.frame(), FIELD_RADIUS);
                items.push(Item::Fill {
                    shape,
                    paint: cx.palette.surface_elevated.into(),
                });
                items.push(Item::Stroke {
                    shape,
                    width: 2.0,
                    paint: cx.palette.accent_color.into(),
                });
            }
            child.paint(cx, items);
        }
        self.painted.set(items.len());
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::Unknown)
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}
