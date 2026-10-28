use crate::text::Paragraph;
use crate::tokens::Color;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn inset(self, amount: f32) -> Self {
        Self::new(
            self.x + amount,
            self.y + amount,
            (self.width - 2.0 * amount).max(0.0),
            (self.height - 2.0 * amount).max(0.0),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Shape {
    pub rect: Rect,
    pub radius: f32,
}

impl Shape {
    pub const fn rect(rect: Rect) -> Self {
        Self { rect, radius: 0.0 }
    }

    pub const fn rounded(rect: Rect, radius: f32) -> Self {
        Self { rect, radius }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stop {
    pub offset: f32,
    pub color: Color,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    Solid(Color),
    LinearGradient {
        start: Point,
        end: Point,
        stops: Vec<Stop>,
    },
}

impl From<Color> for Paint {
    fn from(color: Color) -> Self {
        Self::Solid(color)
    }
}

#[derive(Debug, Clone)]
pub enum Item {
    Fill {
        shape: Shape,
        paint: Paint,
    },
    Stroke {
        shape: Shape,
        width: f32,
        paint: Paint,
    },
    Shadow {
        shape: Shape,
        offset: Point,
        blur_radius: f32,
        color: Color,
    },
    Text {
        origin: Point,
        paragraph: Paragraph,
    },
    Clip {
        shape: Shape,
        items: Vec<Item>,
    },
}

#[derive(Debug, Clone)]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    pub background: Color,
    pub items: Vec<Item>,
}
