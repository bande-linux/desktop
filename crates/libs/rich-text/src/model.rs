use std::ops::Range;
use std::sync::Arc;

use bande_design::Color;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextAttributes {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub family: Option<Arc<str>>,
    pub size: Option<f32>,
    pub color: Option<Color>,
    pub link: Option<Arc<str>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Alignment {
    #[default]
    Start,
    Center,
    End,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListMarker {
    Bullet,
    Number,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ParagraphAttributes {
    pub alignment: Alignment,
    pub list: Option<ListMarker>,
    pub level: u8,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RichText {
    pub text: String,
    pub runs: Vec<(Range<usize>, TextAttributes)>,
    pub paragraphs: Vec<ParagraphAttributes>,
}
