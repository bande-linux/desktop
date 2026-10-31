use crate::{TextRole, TextSize};

pub struct Spacing;

impl Spacing {
    pub const XXS: u32 = 2;
    pub const XS: u32 = 4;
    pub const S: u32 = 8;
    pub const M: u32 = 12;
    pub const L: u32 = 16;
    pub const XL: u32 = 24;
    pub const XXL: u32 = 32;
    pub const XXXL: u32 = 48;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ControlSize {
    Small,
    #[default]
    Regular,
    Large,
}

impl ControlSize {
    pub const fn height(self) -> f32 {
        match self {
            Self::Small => 22.0,
            Self::Regular => 28.0,
            Self::Large => 36.0,
        }
    }

    pub fn height_for(self, text: TextSize) -> f32 {
        self.height() + grown_line(text)
    }
}

pub fn grown_line(text: TextSize) -> f32 {
    (TextRole::Body.spec().line_height * (text.scale() - 1.0)).round()
}
