mod cascade;
mod compute;
mod matching;
mod stylist;

use std::sync::Arc;

pub use cascade::{Restyle, Styles};
pub use stylist::{Stylist, Viewport};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const TRANSPARENT: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Display {
    None,
    Contents,
    Block,
    Inline,
    InlineBlock,
    ListItem,
    Flex,
    InlineFlex,
}

impl Display {
    pub fn is_inline_level(self) -> bool {
        matches!(
            self,
            Display::Inline | Display::InlineBlock | Display::InlineFlex
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Float {
    None,
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Clear {
    None,
    Left,
    Right,
    Both,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FlexDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Align {
    Auto,
    Normal,
    Start,
    End,
    Center,
    Stretch,
    Baseline,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ObjectFit {
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Direction {
    Ltr,
    Rtl,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FontFamily {
    Serif,
    SansSerif,
    Monospace,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WhiteSpace {
    Normal,
    Pre,
    Nowrap,
    PreWrap,
    PreLine,
}

impl WhiteSpace {
    pub fn collapses_spaces(self) -> bool {
        matches!(self, Self::Normal | Self::Nowrap | Self::PreLine)
    }

    pub fn preserves_newlines(self) -> bool {
        !matches!(self, Self::Normal | Self::Nowrap)
    }

    pub fn wraps(self) -> bool {
        !matches!(self, Self::Pre | Self::Nowrap)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TextAlign {
    Start,
    End,
    Left,
    Right,
    Center,
    Justify,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ListStyleType {
    None,
    Disc,
    Circle,
    Square,
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LineHeight {
    Normal,
    Number(f32),
    Px(f32),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum VerticalAlign {
    Baseline,
    Sub,
    Super,
    Middle,
    Top,
    Bottom,
    TextTop,
    TextBottom,
    Length(LengthPercentage),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct TextDecorations {
    pub underline: Option<Color>,
    pub overline: Option<Color>,
    pub line_through: Option<Color>,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct LengthPercentage {
    pub px: f32,
    pub percent: f32,
}

impl LengthPercentage {
    pub const ZERO: LengthPercentage = LengthPercentage {
        px: 0.0,
        percent: 0.0,
    };

    pub const fn px(px: f32) -> Self {
        LengthPercentage { px, percent: 0.0 }
    }

    pub fn resolve(self, basis: f32) -> f32 {
        self.px + self.percent / 100.0 * basis
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Sides<T = f32> {
    pub top: T,
    pub right: T,
    pub bottom: T,
    pub left: T,
}

impl<T: Copy> Sides<T> {
    pub const fn all(value: T) -> Self {
        Sides {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

    pub fn map<U>(self, f: impl Fn(T) -> U) -> Sides<U> {
        Sides {
            top: f(self.top),
            right: f(self.right),
            bottom: f(self.bottom),
            left: f(self.left),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct InheritedStyle {
    pub color: Color,
    pub font_family: FontFamily,
    pub font_size: f32,
    pub font_size_keyword: Option<f32>,
    pub font_weight: u16,
    pub italic: bool,
    pub line_height: LineHeight,
    pub white_space: WhiteSpace,
    pub direction: Direction,
    pub text_align: TextAlign,
    pub list_style_type: ListStyleType,
    pub decorations: TextDecorations,
    pub visible: bool,
    pub custom: Arc<[(Arc<str>, Arc<str>)]>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ResetStyle {
    pub display: Display,
    pub margin: Sides<Option<LengthPercentage>>,
    pub padding: Sides<LengthPercentage>,
    pub border: Sides,
    pub border_color: Sides<Color>,
    pub background_color: Color,
    pub vertical_align: VerticalAlign,
    pub width: Option<LengthPercentage>,
    pub height: Option<LengthPercentage>,
    pub min_width: LengthPercentage,
    pub max_width: Option<LengthPercentage>,
    pub min_height: LengthPercentage,
    pub max_height: Option<LengthPercentage>,
    pub border_box: bool,
    pub text_decoration: TextDecorations,
    pub position: Position,
    pub inset: Sides<Option<LengthPercentage>>,
    pub float: Float,
    pub clear: Clear,
    pub z_index: Option<i32>,
    pub clips: bool,
    pub opacity: f32,
    pub flex_direction: FlexDirection,
    pub flex_wrap: bool,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Option<LengthPercentage>,
    pub order: i32,
    pub justify_content: Align,
    pub align_items: Align,
    pub align_self: Align,
    pub align_content: Align,
    pub row_gap: LengthPercentage,
    pub column_gap: LengthPercentage,
    pub background_image: Option<Arc<str>>,
    pub object_fit: ObjectFit,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ComputedStyle {
    pub inherited: Arc<InheritedStyle>,
    pub reset: Arc<ResetStyle>,
}

pub(crate) const MEDIUM: f32 = 16.0;

impl InheritedStyle {
    pub(crate) fn initial() -> Self {
        InheritedStyle {
            color: Color::BLACK,
            font_family: FontFamily::Serif,
            font_size: MEDIUM,
            font_size_keyword: Some(1.0),
            font_weight: 400,
            italic: false,
            line_height: LineHeight::Normal,
            white_space: WhiteSpace::Normal,
            direction: Direction::Ltr,
            text_align: TextAlign::Start,
            list_style_type: ListStyleType::Disc,
            decorations: TextDecorations::default(),
            visible: true,
            custom: Arc::new([]),
        }
    }
}

impl ResetStyle {
    pub(crate) fn initial() -> Self {
        ResetStyle {
            display: Display::Inline,
            margin: Sides::all(Some(LengthPercentage::ZERO)),
            padding: Sides::all(LengthPercentage::ZERO),
            border: Sides::all(0.0),
            border_color: Sides::all(Color::BLACK),
            background_color: Color::TRANSPARENT,
            vertical_align: VerticalAlign::Baseline,
            width: None,
            height: None,
            min_width: LengthPercentage::ZERO,
            max_width: None,
            min_height: LengthPercentage::ZERO,
            max_height: None,
            border_box: false,
            text_decoration: TextDecorations::default(),
            position: Position::Static,
            inset: Sides::all(None),
            float: Float::None,
            clear: Clear::None,
            z_index: None,
            clips: false,
            opacity: 1.0,
            flex_direction: FlexDirection::Row,
            flex_wrap: false,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: None,
            order: 0,
            justify_content: Align::Normal,
            align_items: Align::Normal,
            align_self: Align::Auto,
            align_content: Align::Normal,
            row_gap: LengthPercentage::ZERO,
            column_gap: LengthPercentage::ZERO,
            background_image: None,
            object_fit: ObjectFit::Fill,
        }
    }
}

impl ComputedStyle {
    pub fn display(&self) -> Display {
        self.reset.display
    }

    pub fn same_layout(&self, other: &ComputedStyle) -> bool {
        let neutral_inherited = |style: &InheritedStyle| InheritedStyle {
            color: Color::BLACK,
            decorations: TextDecorations::default(),
            visible: true,
            ..style.clone()
        };
        let neutral_reset = |style: &ResetStyle| ResetStyle {
            background_color: Color::TRANSPARENT,
            border_color: Sides::all(Color::BLACK),
            text_decoration: TextDecorations::default(),
            ..style.clone()
        };
        neutral_inherited(&self.inherited) == neutral_inherited(&other.inherited)
            && neutral_reset(&self.reset) == neutral_reset(&other.reset)
    }

    pub fn is_out_of_flow(&self) -> bool {
        matches!(self.reset.position, Position::Absolute | Position::Fixed)
    }

    pub fn is_positioned(&self) -> bool {
        self.reset.position != Position::Static
    }

    pub fn is_rtl(&self) -> bool {
        self.inherited.direction == Direction::Rtl
    }
}
