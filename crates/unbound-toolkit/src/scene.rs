use crate::tokens::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Fill {
        rect: Rect,
        color: Color,
    },
    Text {
        x: i32,
        baseline: i32,
        size: f32,
        color: Color,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    pub background: Color,
    pub items: Vec<Item>,
}
