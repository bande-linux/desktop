pub mod calc;
pub mod color;

use crate::parser::{ParseError, ParseResult, Parser};
use crate::tokenizer::Token;
use calc::{CalcLeaf, CalcNode, CalcParser, Category, is_math_function};

macro_rules! units {
    ($name:ident { $($variant:ident = $text:literal,)* }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name { $($variant,)* }

        impl $name {
            pub fn from_name(name: &str) -> Option<Self> {
                $(if name.eq_ignore_ascii_case($text) { return Some($name::$variant); })*
                None
            }

            pub fn name(self) -> &'static str {
                match self { $($name::$variant => $text,)* }
            }
        }
    };
}

units!(LengthUnit {
    Px = "px",
    Cm = "cm",
    Mm = "mm",
    Q = "q",
    In = "in",
    Pt = "pt",
    Pc = "pc",
    Em = "em",
    Rem = "rem",
    Ex = "ex",
    Rex = "rex",
    Ch = "ch",
    Rch = "rch",
    Cap = "cap",
    Rcap = "rcap",
    Ic = "ic",
    Ric = "ric",
    Lh = "lh",
    Rlh = "rlh",
    Vw = "vw",
    Vh = "vh",
    Vi = "vi",
    Vb = "vb",
    Vmin = "vmin",
    Vmax = "vmax",
    Svw = "svw",
    Svh = "svh",
    Svi = "svi",
    Svb = "svb",
    Svmin = "svmin",
    Svmax = "svmax",
    Lvw = "lvw",
    Lvh = "lvh",
    Lvi = "lvi",
    Lvb = "lvb",
    Lvmin = "lvmin",
    Lvmax = "lvmax",
    Dvw = "dvw",
    Dvh = "dvh",
    Dvi = "dvi",
    Dvb = "dvb",
    Dvmin = "dvmin",
    Dvmax = "dvmax",
    Cqw = "cqw",
    Cqh = "cqh",
    Cqi = "cqi",
    Cqb = "cqb",
    Cqmin = "cqmin",
    Cqmax = "cqmax",
});

impl LengthUnit {
    pub fn px_per_unit(self) -> Option<f32> {
        Some(match self {
            LengthUnit::Px => 1.0,
            LengthUnit::Cm => 96.0 / 2.54,
            LengthUnit::Mm => 96.0 / 25.4,
            LengthUnit::Q => 96.0 / 101.6,
            LengthUnit::In => 96.0,
            LengthUnit::Pt => 96.0 / 72.0,
            LengthUnit::Pc => 16.0,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Length {
    pub value: f32,
    pub unit: LengthUnit,
}

impl Length {
    pub const ZERO: Length = Length {
        value: 0.0,
        unit: LengthUnit::Px,
    };

    pub fn px(value: f32) -> Length {
        Length {
            value,
            unit: LengthUnit::Px,
        }
    }

    pub fn to_px(self) -> Option<f32> {
        self.unit.px_per_unit().map(|factor| self.value * factor)
    }
}

pub fn angle_to_degrees(value: f32, unit: &str) -> Option<f32> {
    let factor = match unit.to_ascii_lowercase().as_str() {
        "deg" => 1.0,
        "grad" => 0.9,
        "rad" => 180.0 / std::f32::consts::PI,
        "turn" => 360.0,
        _ => return None,
    };
    Some(value * factor)
}

pub fn time_to_seconds(value: f32, unit: &str) -> Option<f32> {
    match unit.to_ascii_lowercase().as_str() {
        "s" => Some(value),
        "ms" => Some(value / 1000.0),
        _ => None,
    }
}

pub fn resolution_to_dppx(value: f32, unit: &str) -> Option<f32> {
    match unit.to_ascii_lowercase().as_str() {
        "dppx" | "x" => Some(value),
        "dpi" => Some(value / 96.0),
        "dpcm" => Some(value * 2.54 / 96.0),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Range {
    All,
    NonNegative,
}

impl Range {
    fn check(self, value: f32) -> ParseResult<f32> {
        match self {
            Range::NonNegative if value < 0.0 => Err(ParseError::Invalid),
            _ => Ok(value),
        }
    }
}

fn parse_calc(
    input: &mut Parser<'_, '_>,
    name: &str,
    category: Category,
    percentage_as: Option<Category>,
) -> ParseResult<CalcNode> {
    CalcParser { percentage_as }.parse_function(name, input, category)
}

pub fn parse_number(input: &mut Parser<'_, '_>, range: Range) -> ParseResult<f32> {
    match input.next_token()? {
        Token::Number(n) => range.check(n.value as f32),
        Token::Function(name) if is_math_function(&name) => {
            match parse_calc(input, &name, Category::Number, None)?.as_leaf() {
                Some(CalcLeaf::Number(v)) => Ok(if range == Range::NonNegative {
                    v.max(0.0)
                } else {
                    v
                }),
                _ => Err(ParseError::Invalid),
            }
        }
        _ => Err(ParseError::Invalid),
    }
}

pub fn parse_integer(input: &mut Parser<'_, '_>) -> ParseResult<i32> {
    match input.next_token()? {
        Token::Number(n) => n.int_value().ok_or(ParseError::Invalid),
        Token::Function(name) if is_math_function(&name) => {
            match parse_calc(input, &name, Category::Number, None)?.as_leaf() {
                Some(CalcLeaf::Number(v)) => {
                    Ok(v.round().clamp(i32::MIN as f32, i32::MAX as f32) as i32)
                }
                _ => Err(ParseError::Invalid),
            }
        }
        _ => Err(ParseError::Invalid),
    }
}

pub fn parse_percentage(input: &mut Parser<'_, '_>, range: Range) -> ParseResult<f32> {
    match input.next_token()? {
        Token::Percentage(n) => range.check(n.value as f32),
        Token::Function(name) if is_math_function(&name) => {
            match parse_calc(input, &name, Category::Percentage, None)?.as_leaf() {
                Some(CalcLeaf::Percentage(v)) => Ok(v),
                _ => Err(ParseError::Invalid),
            }
        }
        _ => Err(ParseError::Invalid),
    }
}

pub fn parse_angle(input: &mut Parser<'_, '_>, allow_unitless_zero: bool) -> ParseResult<f32> {
    match input.next_token()? {
        Token::Dimension(n, unit) => {
            angle_to_degrees(n.value as f32, &unit).ok_or(ParseError::Invalid)
        }
        Token::Number(n) if allow_unitless_zero && n.value == 0.0 => Ok(0.0),
        Token::Function(name) if is_math_function(&name) => {
            match parse_calc(input, &name, Category::Angle, None)?.as_leaf() {
                Some(CalcLeaf::Angle(v)) => Ok(v),
                _ => Err(ParseError::Invalid),
            }
        }
        _ => Err(ParseError::Invalid),
    }
}

pub fn parse_time(input: &mut Parser<'_, '_>, range: Range) -> ParseResult<f32> {
    match input.next_token()? {
        Token::Dimension(n, unit) => {
            range.check(time_to_seconds(n.value as f32, &unit).ok_or(ParseError::Invalid)?)
        }
        Token::Function(name) if is_math_function(&name) => {
            match parse_calc(input, &name, Category::Time, None)?.as_leaf() {
                Some(CalcLeaf::Time(v)) => range.check(v),
                _ => Err(ParseError::Invalid),
            }
        }
        _ => Err(ParseError::Invalid),
    }
}

pub fn parse_resolution(input: &mut Parser<'_, '_>) -> ParseResult<f32> {
    match input.next_token()? {
        Token::Dimension(n, unit) => {
            resolution_to_dppx(n.value as f32, &unit).ok_or(ParseError::Invalid)
        }
        _ => Err(ParseError::Invalid),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LengthPercentage {
    Length(Length),
    Percentage(f32),
    Calc(Box<CalcNode>),
}

impl LengthPercentage {
    pub const ZERO: LengthPercentage = LengthPercentage::Length(Length::ZERO);

    pub fn parse(input: &mut Parser<'_, '_>, range: Range) -> ParseResult<Self> {
        Self::parse_inner(input, range, true)
    }

    fn parse_inner(input: &mut Parser<'_, '_>, range: Range, percent: bool) -> ParseResult<Self> {
        match input.next_token()? {
            Token::Dimension(n, unit) => {
                let unit = LengthUnit::from_name(&unit).ok_or(ParseError::Invalid)?;
                range.check(n.value as f32)?;
                Ok(LengthPercentage::Length(Length {
                    value: n.value as f32,
                    unit,
                }))
            }
            Token::Number(n) if n.value == 0.0 => Ok(LengthPercentage::ZERO),
            Token::Percentage(n) if percent => {
                Ok(LengthPercentage::Percentage(range.check(n.value as f32)?))
            }
            Token::Function(name) if is_math_function(&name) => {
                let percentage_as = percent.then_some(Category::Length);
                let node = parse_calc(input, &name, Category::Length, percentage_as)?;
                Ok(match node.as_leaf() {
                    Some(CalcLeaf::Length(l)) => LengthPercentage::Length(Length {
                        value: clamp_range(l.value, range),
                        unit: l.unit,
                    }),
                    Some(CalcLeaf::Percentage(p)) => {
                        LengthPercentage::Percentage(clamp_range(p, range))
                    }
                    _ => {
                        let node = if range == Range::NonNegative {
                            CalcNode::Max(Box::new([node, CalcNode::Leaf(CalcLeaf::Number(0.0))]))
                        } else {
                            node
                        };
                        LengthPercentage::Calc(Box::new(node))
                    }
                })
            }
            _ => Err(ParseError::Invalid),
        }
    }

    pub fn parse_length(input: &mut Parser<'_, '_>, range: Range) -> ParseResult<Self> {
        Self::parse_inner(input, range, false)
    }

    pub fn resolve(&self, resolve_length: &dyn Fn(Length) -> f32, basis: f32) -> f32 {
        match self {
            LengthPercentage::Length(length) => resolve_length(*length),
            LengthPercentage::Percentage(p) => basis * p / 100.0,
            LengthPercentage::Calc(node) => node
                .evaluate(&|leaf| match *leaf {
                    CalcLeaf::Length(length) => Some(resolve_length(length)),
                    CalcLeaf::Percentage(p) => Some(basis * p / 100.0),
                    _ => None,
                })
                .unwrap_or(0.0),
        }
    }
}

fn clamp_range(value: f32, range: Range) -> f32 {
    match range {
        Range::NonNegative => value.max(0.0),
        Range::All => value,
    }
}

pub fn parse_length(input: &mut Parser<'_, '_>, range: Range) -> ParseResult<LengthPercentage> {
    LengthPercentage::parse_length(input, range)
}
