use accesskit::{Node, Role};
use bande_graphics::{Item, Rect, TextSize};

use super::{Context, Size, Widget, WidgetId};

pub struct FreeLayout {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    frames: Vec<Rect>,
    in_header: Vec<bool>,
    frame: Rect,
}

impl Default for FreeLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl FreeLayout {
    pub fn new() -> Self {
        Self {
            id: WidgetId::next(),
            children: Vec::new(),
            frames: Vec::new(),
            in_header: Vec::new(),
            frame: Rect::default(),
        }
    }

    pub fn insert(&mut self, child: impl Widget + 'static, frame: Rect) -> WidgetId {
        self.push(Box::new(child), frame, false)
    }

    pub fn insert_in_header(&mut self, child: impl Widget + 'static, frame: Rect) -> WidgetId {
        self.push(Box::new(child), frame, true)
    }

    fn push(&mut self, child: Box<dyn Widget>, frame: Rect, in_header: bool) -> WidgetId {
        let id = child.id();
        self.children.push(child);
        self.frames.push(frame);
        self.in_header.push(in_header);
        id
    }

    pub fn replace(&mut self, old: WidgetId, child: impl Widget + 'static) -> Option<WidgetId> {
        let index = self.index(old)?;
        let id = child.id();
        self.children[index] = Box::new(child);
        Some(id)
    }

    pub fn remove(&mut self, id: WidgetId) -> Option<Box<dyn Widget>> {
        let index = self.index(id)?;
        self.frames.remove(index);
        self.in_header.remove(index);
        Some(self.children.remove(index))
    }

    pub fn set_frame(&mut self, id: WidgetId, frame: Rect) {
        if let Some(index) = self.index(id) {
            self.frames[index] = frame;
        }
    }

    pub fn frame_of(&self, id: WidgetId) -> Option<Rect> {
        self.index(id)
            .map(|index| self.frames[index])
            .map(|frame| self.absolute(frame))
    }

    fn index(&self, id: WidgetId) -> Option<usize> {
        self.children.iter().position(|child| child.id() == id)
    }

    fn absolute(&self, frame: Rect) -> Rect {
        Rect::new(
            self.frame.x + frame.x,
            self.frame.y + frame.y,
            frame.width,
            frame.height,
        )
    }
}

fn shown(frame: Rect) -> bool {
    frame.width > 0.0 && frame.height > 0.0
}

impl Widget for FreeLayout {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let absolute = self
            .frames
            .iter()
            .map(|frame| self.absolute(*frame))
            .collect::<Vec<_>>();
        let text_size = cx.text.size();
        for ((child, frame), in_header) in
            self.children.iter_mut().zip(absolute).zip(&self.in_header)
        {
            if *in_header {
                cx.text.set_size(TextSize::DEFAULT);
            }
            child.measure(cx, Size::new(frame.width, frame.height));
            child.place(cx, frame);
            cx.text.set_size(text_size);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for (child, frame) in self.children.iter().zip(&self.frames) {
            if shown(*frame) {
                child.paint(cx, items);
            }
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
