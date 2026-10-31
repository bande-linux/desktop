use accesskit::{Node, Role};

use super::{Context, Size, Widget, WidgetId};
use bande_graphics::{Item, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

pub struct Stack {
    id: WidgetId,
    axis: Axis,
    spacing: f32,
    padding: f32,
    children: Vec<Box<dyn Widget>>,
    sizes: Vec<Size>,
    frame: Rect,
}

impl Stack {
    pub fn new(axis: Axis, spacing: f32, padding: f32, children: Vec<Box<dyn Widget>>) -> Self {
        Self {
            id: WidgetId::next(),
            axis,
            spacing,
            padding,
            children,
            sizes: Vec::new(),
            frame: Rect::default(),
        }
    }
}

impl Widget for Stack {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let inner = Size::new(
            (available.width - 2.0 * self.padding).max(0.0),
            (available.height - 2.0 * self.padding).max(0.0),
        );
        self.sizes = self
            .children
            .iter_mut()
            .map(|child| child.measure(cx, inner))
            .collect();
        let gaps = self.spacing * self.sizes.len().saturating_sub(1) as f32;
        let (main, cross) = match self.axis {
            Axis::Horizontal => (
                self.sizes.iter().map(|size| size.width).sum::<f32>() + gaps,
                self.sizes
                    .iter()
                    .map(|size| size.height)
                    .fold(0.0, f32::max),
            ),
            Axis::Vertical => (
                self.sizes.iter().map(|size| size.height).sum::<f32>() + gaps,
                self.sizes.iter().map(|size| size.width).fold(0.0, f32::max),
            ),
        };
        let padded = |value: f32| value + 2.0 * self.padding;
        match self.axis {
            Axis::Horizontal => Size::new(padded(main), padded(cross)),
            Axis::Vertical => Size::new(padded(cross), padded(main)),
        }
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let inner = frame.inset(self.padding);
        let mut offset = 0.0;
        for (child, size) in self.children.iter_mut().zip(&self.sizes) {
            let child_frame = match self.axis {
                Axis::Horizontal => Rect::new(
                    inner.x + offset,
                    inner.y + ((inner.height - size.height) / 2.0).round(),
                    size.width,
                    size.height,
                ),
                Axis::Vertical => Rect::new(inner.x, inner.y + offset, inner.width, size.height),
            };
            child.place(cx, child_frame);
            offset += self.spacing
                + match self.axis {
                    Axis::Horizontal => size.width,
                    Axis::Vertical => size.height,
                };
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
