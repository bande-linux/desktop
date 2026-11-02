use std::fmt::{self, Display as _, Write};

use crate::parser::{ParseError, ParseResult, Parser};
use crate::tokenizer::Token;
use crate::values::color::{Color, write_number};
use crate::values::{
    Length, LengthPercentage, Parse, Range, parse_angle, parse_integer, parse_number,
    parse_percentage,
};

fn write_list<T: fmt::Display>(
    f: &mut fmt::Formatter<'_>,
    items: &[T],
    separator: &str,
) -> fmt::Result {
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            f.write_str(separator)?;
        }
        item.fmt(f)?;
    }
    Ok(())
}

fn write_string(f: &mut impl Write, value: &str) -> fmt::Result {
    f.write_char('"')?;
    for c in value.chars() {
        match c {
            '"' | '\\' => {
                f.write_char('\\')?;
                f.write_char(c)?;
            }
            '\n' => f.write_str("\\a ")?,
            c if c.is_control() => write!(f, "\\{:x} ", c as u32)?,
            c => f.write_char(c)?,
        }
    }
    f.write_char('"')
}

fn write_url(f: &mut fmt::Formatter<'_>, url: &str) -> fmt::Result {
    f.write_str("url(")?;
    write_string(f, url)?;
    f.write_char(')')
}

impl Parse for Color {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        Color::parse(input)
    }
}

impl Parse for LengthPercentage {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        LengthPercentage::parse(input, Range::All)
    }
}

macro_rules! newtype {
    ($name:ident($inner:ty), $parse:expr) => {
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name(pub $inner);

        impl Parse for $name {
            fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
                let parse: fn(&mut Parser<'_, '_>) -> ParseResult<$inner> = $parse;
                parse(input).map($name)
            }
        }
    };
}

newtype!(NonNegativeLengthPercentage(LengthPercentage), |p| {
    LengthPercentage::parse(p, Range::NonNegative)
});
newtype!(SignedLength(LengthPercentage), |p| {
    LengthPercentage::parse_length(p, Range::All)
});
newtype!(NonNegativeNumber(f32), |p| parse_number(
    p,
    Range::NonNegative
));
newtype!(Integer(i32), parse_integer);
newtype!(Opacity(f32), |p| {
    p.try_parse(|p| parse_number(p, Range::All))
        .or_else(|_| parse_percentage(p, Range::All).map(|v| v / 100.0))
        .map(|v| v.clamp(0.0, 1.0))
});

impl fmt::Display for NonNegativeLengthPercentage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for SignedLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for NonNegativeNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_number(f, self.0)
    }
}

impl fmt::Display for Integer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for Opacity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_number(f, self.0)
    }
}

macro_rules! keyword_or {
    ($name:ident { $($keyword:ident = $text:literal,)* } $value:ident($inner:ty) = $parse:expr) => {
        #[derive(Clone, Debug, PartialEq)]
        pub enum $name {
            $($keyword,)*
            $value($inner),
        }

        impl Parse for $name {
            fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
                $(if input.try_ident_matching($text) { return Ok($name::$keyword); })*
                let parse: fn(&mut Parser<'_, '_>) -> ParseResult<$inner> = $parse;
                parse(input).map($name::$value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    $($name::$keyword => f.write_str($text),)*
                    $name::$value(value) => value.fmt(f),
                }
            }
        }
    };
}

keyword_or!(LengthPercentageOrAuto { Auto = "auto", } LengthPercentage(LengthPercentage) = |p| {
    LengthPercentage::parse(p, Range::All)
});
keyword_or!(ZIndex { Auto = "auto", } Integer(Integer) = |p| parse_integer(p).map(Integer));
keyword_or!(Gap { Normal = "normal", } LengthPercentage(LengthPercentage) = |p| {
    LengthPercentage::parse(p, Range::NonNegative)
});
keyword_or!(Spacing { Normal = "normal", } LengthPercentage(LengthPercentage) = |p| {
    LengthPercentage::parse(p, Range::All)
});
keyword_or!(TextDecorationThickness { Auto = "auto", FromFont = "from-font", }
    LengthPercentage(LengthPercentage) = |p| LengthPercentage::parse(p, Range::All));
keyword_or!(MaxSize {
    None = "none", MinContent = "min-content", MaxContent = "max-content", FitContent = "fit-content",
} LengthPercentage(LengthPercentage) = |p| LengthPercentage::parse(p, Range::NonNegative));

#[derive(Clone, Debug, PartialEq)]
pub enum Size {
    Auto,
    MinContent,
    MaxContent,
    FitContent(Option<LengthPercentage>),
    LengthPercentage(LengthPercentage),
}

impl Parse for Size {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        if input.try_ident_matching("auto") {
            return Ok(Size::Auto);
        }
        if input.try_ident_matching("min-content") {
            return Ok(Size::MinContent);
        }
        if input.try_ident_matching("max-content") {
            return Ok(Size::MaxContent);
        }
        if input.try_ident_matching("fit-content") {
            return Ok(Size::FitContent(None));
        }
        if input
            .try_parse(|p| p.expect_function_matching("fit-content"))
            .is_ok()
        {
            return input.parse_nested_block(|p| {
                let limit = LengthPercentage::parse(p, Range::NonNegative)?;
                p.expect_exhausted()?;
                Ok(Size::FitContent(Some(limit)))
            });
        }
        LengthPercentage::parse(input, Range::NonNegative).map(Size::LengthPercentage)
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Size::Auto => f.write_str("auto"),
            Size::MinContent => f.write_str("min-content"),
            Size::MaxContent => f.write_str("max-content"),
            Size::FitContent(None) => f.write_str("fit-content"),
            Size::FitContent(Some(limit)) => write!(f, "fit-content({limit})"),
            Size::LengthPercentage(lp) => lp.fmt(f),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FlexBasis {
    Content,
    Size(Size),
}

impl Parse for FlexBasis {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        if input.try_ident_matching("content") {
            return Ok(FlexBasis::Content);
        }
        Size::parse(input).map(FlexBasis::Size)
    }
}

impl fmt::Display for FlexBasis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlexBasis::Content => f.write_str("content"),
            FlexBasis::Size(size) => size.fmt(f),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BorderWidth(pub LengthPercentage);

impl Parse for BorderWidth {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        for (keyword, px) in [("thin", 1.0), ("medium", 3.0), ("thick", 5.0)] {
            if input.try_ident_matching(keyword) {
                return Ok(BorderWidth(LengthPercentage::Length(Length::px(px))));
            }
        }
        LengthPercentage::parse_length(input, Range::NonNegative).map(BorderWidth)
    }
}

impl fmt::Display for BorderWidth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BorderRadius(pub LengthPercentage, pub LengthPercentage);

impl Parse for BorderRadius {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        let horizontal = LengthPercentage::parse(input, Range::NonNegative)?;
        let vertical = input
            .try_parse(|p| LengthPercentage::parse(p, Range::NonNegative))
            .unwrap_or_else(|_| horizontal.clone());
        Ok(BorderRadius(horizontal, vertical))
    }
}

impl fmt::Display for BorderRadius {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)?;
        if self.1 != self.0 {
            write!(f, " {}", self.1)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BorderSpacing(pub LengthPercentage, pub LengthPercentage);

impl Parse for BorderSpacing {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        let horizontal = LengthPercentage::parse_length(input, Range::NonNegative)?;
        let vertical = input
            .try_parse(|p| LengthPercentage::parse_length(p, Range::NonNegative))
            .unwrap_or_else(|_| horizontal.clone());
        Ok(BorderSpacing(horizontal, vertical))
    }
}

impl fmt::Display for BorderSpacing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)?;
        if self.1 != self.0 {
            write!(f, " {}", self.1)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display {
    None,
    Contents,
    Block,
    FlowRoot,
    Inline,
    InlineBlock,
    RunIn,
    ListItem,
    InlineListItem,
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
    Table,
    InlineTable,
    TableRowGroup,
    TableHeaderGroup,
    TableFooterGroup,
    TableRow,
    TableCell,
    TableColumnGroup,
    TableColumn,
    TableCaption,
    Ruby,
    RubyText,
}

const DISPLAY_KEYWORDS: &[(&str, Display)] = &[
    ("none", Display::None),
    ("contents", Display::Contents),
    ("block", Display::Block),
    ("flow-root", Display::FlowRoot),
    ("inline", Display::Inline),
    ("inline-block", Display::InlineBlock),
    ("run-in", Display::RunIn),
    ("list-item", Display::ListItem),
    ("inline-list-item", Display::InlineListItem),
    ("flex", Display::Flex),
    ("inline-flex", Display::InlineFlex),
    ("grid", Display::Grid),
    ("inline-grid", Display::InlineGrid),
    ("table", Display::Table),
    ("inline-table", Display::InlineTable),
    ("table-row-group", Display::TableRowGroup),
    ("table-header-group", Display::TableHeaderGroup),
    ("table-footer-group", Display::TableFooterGroup),
    ("table-row", Display::TableRow),
    ("table-cell", Display::TableCell),
    ("table-column-group", Display::TableColumnGroup),
    ("table-column", Display::TableColumn),
    ("table-caption", Display::TableCaption),
    ("ruby", Display::Ruby),
    ("ruby-text", Display::RubyText),
    ("-webkit-box", Display::Flex),
    ("-webkit-inline-box", Display::InlineFlex),
    ("-webkit-flex", Display::Flex),
    ("-webkit-inline-flex", Display::InlineFlex),
];

const DISPLAY_PAIRS: &[(&str, &str, Display)] = &[
    ("block", "flow", Display::Block),
    ("block", "flow-root", Display::FlowRoot),
    ("inline", "flow", Display::Inline),
    ("inline", "flow-root", Display::InlineBlock),
    ("run-in", "flow", Display::RunIn),
    ("block", "flex", Display::Flex),
    ("inline", "flex", Display::InlineFlex),
    ("block", "grid", Display::Grid),
    ("inline", "grid", Display::InlineGrid),
    ("block", "table", Display::Table),
    ("inline", "table", Display::InlineTable),
    ("block", "list-item", Display::ListItem),
    ("list-item", "block", Display::ListItem),
    ("inline", "list-item", Display::InlineListItem),
    ("list-item", "inline", Display::InlineListItem),
    ("inline", "ruby", Display::Ruby),
];

impl Display {
    pub fn name(self) -> &'static str {
        DISPLAY_KEYWORDS
            .iter()
            .find(|(_, d)| *d == self)
            .map_or("inline", |(name, _)| name)
    }
}

impl Parse for Display {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        let first = input.expect_ident()?;
        let second = input.try_parse(|p| p.expect_ident()).ok();
        match second {
            None => DISPLAY_KEYWORDS
                .iter()
                .find(|(name, _)| first.eq_ignore_ascii_case(name))
                .map(|(_, d)| *d)
                .ok_or(ParseError::Invalid),
            Some(second) => DISPLAY_PAIRS
                .iter()
                .find(|(a, b, _)| {
                    (first.eq_ignore_ascii_case(a) && second.eq_ignore_ascii_case(b))
                        || (first.eq_ignore_ascii_case(b)
                            && second.eq_ignore_ascii_case(a)
                            && *a != "list-item"
                            && *b != "list-item")
                })
                .map(|(_, _, d)| *d)
                .ok_or(ParseError::Invalid),
        }
    }
}

impl fmt::Display for Display {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum GenericFamily {
    Serif,
    SansSerif,
    Monospace,
    Cursive,
    Fantasy,
    SystemUi,
    UiSerif,
    UiSansSerif,
    UiMonospace,
    UiRounded,
    Math,
    Emoji,
}

const GENERIC_FAMILIES: &[(&str, GenericFamily)] = &[
    ("serif", GenericFamily::Serif),
    ("sans-serif", GenericFamily::SansSerif),
    ("monospace", GenericFamily::Monospace),
    ("cursive", GenericFamily::Cursive),
    ("fantasy", GenericFamily::Fantasy),
    ("system-ui", GenericFamily::SystemUi),
    ("ui-serif", GenericFamily::UiSerif),
    ("ui-sans-serif", GenericFamily::UiSansSerif),
    ("ui-monospace", GenericFamily::UiMonospace),
    ("ui-rounded", GenericFamily::UiRounded),
    ("math", GenericFamily::Math),
    ("emoji", GenericFamily::Emoji),
    ("-apple-system", GenericFamily::SystemUi),
];

#[derive(Clone, Debug, PartialEq)]
pub enum FamilyName {
    Named(Box<str>),
    Generic(GenericFamily),
}

impl fmt::Display for FamilyName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FamilyName::Named(name) => write_string(f, name),
            FamilyName::Generic(generic) => f.write_str(
                GENERIC_FAMILIES
                    .iter()
                    .find(|(_, g)| g == generic)
                    .map_or("serif", |(name, _)| name),
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FontFamily(pub Box<[FamilyName]>);

impl FamilyName {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        if let Ok(name) = input.try_parse(|p| p.expect_string()) {
            return Ok(FamilyName::Named(name.into()));
        }
        let first = input.expect_ident()?;
        let reserved = [
            "inherit",
            "initial",
            "unset",
            "default",
            "revert",
            "revert-layer",
        ];
        if reserved.iter().any(|r| first.eq_ignore_ascii_case(r)) {
            return Err(ParseError::Invalid);
        }
        let mut name = first.into_owned();
        let mut single = true;
        while let Ok(ident) = input.try_parse(|p| p.expect_ident()) {
            single = false;
            name.push(' ');
            name.push_str(&ident);
        }
        if single
            && let Some((_, generic)) = GENERIC_FAMILIES
                .iter()
                .find(|(g, _)| name.eq_ignore_ascii_case(g))
        {
            return Ok(FamilyName::Generic(generic.clone()));
        }
        Ok(FamilyName::Named(name.into()))
    }
}

impl Parse for FontFamily {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        input
            .parse_comma_separated(FamilyName::parse)
            .map(|names| FontFamily(names.into_boxed_slice()))
    }
}

impl fmt::Display for FontFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_list(f, &self.0, ", ")
    }
}

keyword_or!(FontSize {
    XxSmall = "xx-small", XSmall = "x-small", Small = "small", Medium = "medium", Large = "large",
    XLarge = "x-large", XxLarge = "xx-large", XxxLarge = "xxx-large", Larger = "larger",
    Smaller = "smaller", Math = "math",
} LengthPercentage(LengthPercentage) = |p| LengthPercentage::parse(p, Range::NonNegative));

#[derive(Clone, Debug, PartialEq)]
pub enum FontWeight {
    Absolute(f32),
    Bolder,
    Lighter,
}

impl Parse for FontWeight {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        if input.try_ident_matching("normal") {
            return Ok(FontWeight::Absolute(400.0));
        }
        if input.try_ident_matching("bold") {
            return Ok(FontWeight::Absolute(700.0));
        }
        if input.try_ident_matching("bolder") {
            return Ok(FontWeight::Bolder);
        }
        if input.try_ident_matching("lighter") {
            return Ok(FontWeight::Lighter);
        }
        let weight = parse_number(input, Range::All)?;
        if (1.0..=1000.0).contains(&weight) {
            Ok(FontWeight::Absolute(weight))
        } else {
            Err(ParseError::Invalid)
        }
    }
}

impl fmt::Display for FontWeight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FontWeight::Absolute(weight) => write_number(f, *weight),
            FontWeight::Bolder => f.write_str("bolder"),
            FontWeight::Lighter => f.write_str("lighter"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FontStretch(pub f32);

const FONT_STRETCH_KEYWORDS: &[(&str, f32)] = &[
    ("normal", 100.0),
    ("ultra-condensed", 50.0),
    ("extra-condensed", 62.5),
    ("condensed", 75.0),
    ("semi-condensed", 87.5),
    ("semi-expanded", 112.5),
    ("expanded", 125.0),
    ("extra-expanded", 150.0),
    ("ultra-expanded", 200.0),
];

impl Parse for FontStretch {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        if let Ok(ident) = input.try_parse(|p| p.expect_ident()) {
            return FONT_STRETCH_KEYWORDS
                .iter()
                .find(|(name, _)| ident.eq_ignore_ascii_case(name))
                .map(|(_, v)| FontStretch(*v))
                .ok_or(ParseError::Invalid);
        }
        parse_percentage(input, Range::NonNegative).map(FontStretch)
    }
}

impl fmt::Display for FontStretch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match FONT_STRETCH_KEYWORDS.iter().find(|(_, v)| *v == self.0) {
            Some((name, _)) => f.write_str(name),
            None => {
                write_number(f, self.0)?;
                f.write_char('%')
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LineHeight {
    Normal,
    Number(f32),
    LengthPercentage(LengthPercentage),
}

impl Parse for LineHeight {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        if input.try_ident_matching("normal") {
            return Ok(LineHeight::Normal);
        }
        if let Ok(number) = input.try_parse(|p| parse_number(p, Range::NonNegative)) {
            return Ok(LineHeight::Number(number));
        }
        LengthPercentage::parse(input, Range::NonNegative).map(LineHeight::LengthPercentage)
    }
}

impl fmt::Display for LineHeight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LineHeight::Normal => f.write_str("normal"),
            LineHeight::Number(n) => write_number(f, *n),
            LineHeight::LengthPercentage(lp) => lp.fmt(f),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextDecorationLine(pub u8);

impl TextDecorationLine {
    pub const UNDERLINE: u8 = 1;
    pub const OVERLINE: u8 = 2;
    pub const LINE_THROUGH: u8 = 4;
    pub const BLINK: u8 = 8;
    const NAMES: [(&str, u8); 4] = [
        ("underline", Self::UNDERLINE),
        ("overline", Self::OVERLINE),
        ("line-through", Self::LINE_THROUGH),
        ("blink", Self::BLINK),
    ];
}

impl Parse for TextDecorationLine {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        if input.try_ident_matching("none") {
            return Ok(TextDecorationLine(0));
        }
        let mut bits = 0;
        while let Ok(bit) = input.try_parse(|p| {
            let ident = p.expect_ident()?;
            Self::NAMES
                .iter()
                .find(|(name, _)| ident.eq_ignore_ascii_case(name))
                .map(|(_, bit)| *bit)
                .ok_or(ParseError::Invalid)
        }) {
            if bits & bit != 0 {
                return Err(ParseError::Invalid);
            }
            bits |= bit;
        }
        if bits == 0 {
            return Err(ParseError::Invalid);
        }
        Ok(TextDecorationLine(bits))
    }
}

impl fmt::Display for TextDecorationLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            return f.write_str("none");
        }
        let names: Vec<&str> = Self::NAMES
            .iter()
            .filter(|(_, bit)| self.0 & bit != 0)
            .map(|(name, _)| *name)
            .collect();
        f.write_str(&names.join(" "))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shadow {
    pub color: Option<Color>,
    pub x: LengthPercentage,
    pub y: LengthPercentage,
    pub blur: LengthPercentage,
    pub spread: LengthPercentage,
    pub inset: bool,
}

impl Shadow {
    fn parse(input: &mut Parser<'_, '_>, box_shadow: bool) -> ParseResult<Shadow> {
        let mut color = None;
        let mut lengths: Option<Vec<LengthPercentage>> = None;
        let mut inset = false;
        loop {
            if box_shadow && !inset && input.try_ident_matching("inset") {
                inset = true;
                continue;
            }
            if color.is_none()
                && let Ok(c) = input.try_parse(Color::parse)
            {
                color = Some(c);
                continue;
            }
            if lengths.is_none()
                && let Ok(first) =
                    input.try_parse(|p| LengthPercentage::parse_length(p, Range::All))
            {
                let mut values = vec![first, LengthPercentage::parse_length(input, Range::All)?];
                if let Ok(blur) =
                    input.try_parse(|p| LengthPercentage::parse_length(p, Range::NonNegative))
                {
                    values.push(blur);
                    if box_shadow
                        && let Ok(spread) =
                            input.try_parse(|p| LengthPercentage::parse_length(p, Range::All))
                    {
                        values.push(spread);
                    }
                }
                lengths = Some(values);
                continue;
            }
            break;
        }
        let mut lengths = lengths.ok_or(ParseError::Invalid)?.into_iter();
        let mut next = || lengths.next().unwrap_or(LengthPercentage::ZERO);
        Ok(Shadow {
            color,
            x: next(),
            y: next(),
            blur: next(),
            spread: next(),
            inset,
        })
    }
}

impl fmt::Display for Shadow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(color) = &self.color {
            write!(f, "{color} ")?;
        }
        write!(f, "{} {}", self.x, self.y)?;
        if self.blur != LengthPercentage::ZERO || self.spread != LengthPercentage::ZERO {
            write!(f, " {}", self.blur)?;
        }
        if self.spread != LengthPercentage::ZERO {
            write!(f, " {}", self.spread)?;
        }
        if self.inset {
            f.write_str(" inset")?;
        }
        Ok(())
    }
}

macro_rules! shadow_list {
    ($name:ident, $box_shadow:expr) => {
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name(pub Box<[Shadow]>);

        impl Parse for $name {
            fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
                if input.try_ident_matching("none") {
                    return Ok($name(Box::new([])));
                }
                input
                    .parse_comma_separated(|p| Shadow::parse(p, $box_shadow))
                    .map(|shadows| $name(shadows.into_boxed_slice()))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                if self.0.is_empty() {
                    return f.write_str("none");
                }
                write_list(f, &self.0, ", ")
            }
        }
    };
}

shadow_list!(BoxShadow, true);
shadow_list!(TextShadow, false);

#[derive(Clone, Debug, PartialEq)]
pub enum TabSize {
    Number(f32),
    Length(LengthPercentage),
}

impl Parse for TabSize {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        if let Ok(n) = input.try_parse(|p| parse_number(p, Range::NonNegative)) {
            return Ok(TabSize::Number(n));
        }
        LengthPercentage::parse_length(input, Range::NonNegative).map(TabSize::Length)
    }
}

impl fmt::Display for TabSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TabSize::Number(n) => write_number(f, *n),
            TabSize::Length(l) => l.fmt(f),
        }
    }
}

keyword_or!(VerticalAlign {
    Baseline = "baseline", Sub = "sub", Super = "super", TextTop = "text-top",
    TextBottom = "text-bottom", Middle = "middle", Top = "top", Bottom = "bottom",
} LengthPercentage(LengthPercentage) = |p| LengthPercentage::parse(p, Range::All));

#[derive(Clone, Debug, PartialEq)]
pub enum ListStyleType {
    None,
    CounterStyle(Box<str>),
    String(Box<str>),
}

impl Parse for ListStyleType {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        if let Ok(string) = input.try_parse(|p| p.expect_string()) {
            return Ok(ListStyleType::String(string.into()));
        }
        let ident = input.expect_ident()?;
        if ident.eq_ignore_ascii_case("none") {
            return Ok(ListStyleType::None);
        }
        Ok(ListStyleType::CounterStyle(
            ident.to_ascii_lowercase().into(),
        ))
    }
}

impl fmt::Display for ListStyleType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ListStyleType::None => f.write_str("none"),
            ListStyleType::CounterStyle(name) => f.write_str(name),
            ListStyleType::String(value) => write_string(f, value),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ContentItem {
    String(Box<str>),
    Url(Box<str>),
    Attr(Box<str>),
    Counter(Box<str>, ListStyleType),
    Counters(Box<str>, Box<str>, ListStyleType),
    OpenQuote,
    CloseQuote,
    NoOpenQuote,
    NoCloseQuote,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Normal,
    None,
    Items(Box<[ContentItem]>, Option<Box<str>>),
}

fn parse_counter_style(input: &mut Parser<'_, '_>) -> ParseResult<ListStyleType> {
    if input.try_parse(|p| p.expect_comma()).is_ok() {
        ListStyleType::parse(input)
    } else {
        Ok(ListStyleType::CounterStyle("decimal".into()))
    }
}

impl ContentItem {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<ContentItem> {
        match input.next_token()? {
            Token::String(value) => Ok(ContentItem::String(value.into())),
            Token::Url(url) => Ok(ContentItem::Url(url.into())),
            Token::Ident(ident) => {
                let lower = ident.to_ascii_lowercase();
                Ok(match lower.as_str() {
                    "open-quote" => ContentItem::OpenQuote,
                    "close-quote" => ContentItem::CloseQuote,
                    "no-open-quote" => ContentItem::NoOpenQuote,
                    "no-close-quote" => ContentItem::NoCloseQuote,
                    _ => return Err(ParseError::Invalid),
                })
            }
            Token::Function(name) => {
                let name = name.to_ascii_lowercase();
                input.parse_nested_block(|p| {
                    let item = match name.as_str() {
                        "url" => ContentItem::Url(p.expect_string()?.into()),
                        "attr" => ContentItem::Attr(p.expect_ident()?.into()),
                        "counter" => {
                            let counter = p.expect_ident()?.into();
                            ContentItem::Counter(counter, parse_counter_style(p)?)
                        }
                        "counters" => {
                            let counter = p.expect_ident()?.into();
                            p.expect_comma()?;
                            let separator = p.expect_string()?.into();
                            ContentItem::Counters(counter, separator, parse_counter_style(p)?)
                        }
                        _ => return Err(ParseError::Invalid),
                    };
                    p.expect_exhausted()?;
                    Ok(item)
                })
            }
            _ => Err(ParseError::Invalid),
        }
    }
}

impl fmt::Display for ContentItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContentItem::String(value) => write_string(f, value),
            ContentItem::Url(url) => write_url(f, url),
            ContentItem::Attr(name) => write!(f, "attr({name})"),
            ContentItem::Counter(name, style) => write!(f, "counter({name}, {style})"),
            ContentItem::Counters(name, separator, style) => {
                write!(f, "counters({name}, ")?;
                write_string(f, separator)?;
                write!(f, ", {style})")
            }
            ContentItem::OpenQuote => f.write_str("open-quote"),
            ContentItem::CloseQuote => f.write_str("close-quote"),
            ContentItem::NoOpenQuote => f.write_str("no-open-quote"),
            ContentItem::NoCloseQuote => f.write_str("no-close-quote"),
        }
    }
}

impl Parse for Content {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        if input.try_ident_matching("normal") {
            return Ok(Content::Normal);
        }
        if input.try_ident_matching("none") {
            return Ok(Content::None);
        }
        let mut items = vec![ContentItem::parse(input)?];
        while let Ok(item) = input.try_parse(ContentItem::parse) {
            items.push(item);
        }
        let alt = if input.try_delim('/') {
            Some(input.expect_string()?.into())
        } else {
            None
        };
        Ok(Content::Items(items.into_boxed_slice(), alt))
    }
}

impl fmt::Display for Content {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Content::Normal => f.write_str("normal"),
            Content::None => f.write_str("none"),
            Content::Items(items, alt) => {
                write_list(f, items, " ")?;
                if let Some(alt) = alt {
                    f.write_str(" / ")?;
                    write_string(f, alt)?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Quotes {
    Auto,
    None,
    Pairs(Box<[(Box<str>, Box<str>)]>),
}

impl Parse for Quotes {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        if input.try_ident_matching("auto") {
            return Ok(Quotes::Auto);
        }
        if input.try_ident_matching("none") {
            return Ok(Quotes::None);
        }
        let mut pairs = vec![];
        while let Ok(open) = input.try_parse(|p| p.expect_string()) {
            pairs.push((open.into(), input.expect_string()?.into()));
        }
        if pairs.is_empty() {
            return Err(ParseError::Invalid);
        }
        Ok(Quotes::Pairs(pairs.into_boxed_slice()))
    }
}

impl fmt::Display for Quotes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Quotes::Auto => f.write_str("auto"),
            Quotes::None => f.write_str("none"),
            Quotes::Pairs(pairs) => {
                for (i, (open, close)) in pairs.iter().enumerate() {
                    if i > 0 {
                        f.write_char(' ')?;
                    }
                    write_string(f, open)?;
                    f.write_char(' ')?;
                    write_string(f, close)?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Counters(pub Box<[(Box<str>, Option<i32>)]>);

impl Parse for Counters {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        if input.try_ident_matching("none") {
            return Ok(Counters(Box::new([])));
        }
        let mut counters = vec![];
        while let Ok(name) = input.try_parse(|p| p.expect_ident()) {
            let value = input.try_parse(parse_integer).ok();
            counters.push((Box::<str>::from(name), value));
        }
        if counters.is_empty() {
            return Err(ParseError::Invalid);
        }
        Ok(Counters(counters.into_boxed_slice()))
    }
}

impl fmt::Display for Counters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return f.write_str("none");
        }
        for (i, (name, value)) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_char(' ')?;
            }
            f.write_str(name)?;
            if let Some(value) = value {
                write!(f, " {value}")?;
            }
        }
        Ok(())
    }
}

pub type CursorImage = (Box<str>, Option<(f32, f32)>);

#[derive(Clone, Debug, PartialEq)]
pub struct Cursor {
    pub images: Box<[CursorImage]>,
    pub keyword: Box<str>,
}

const CURSOR_KEYWORDS: &[&str] = &[
    "auto",
    "default",
    "none",
    "context-menu",
    "help",
    "pointer",
    "progress",
    "wait",
    "cell",
    "crosshair",
    "text",
    "vertical-text",
    "alias",
    "copy",
    "move",
    "no-drop",
    "not-allowed",
    "grab",
    "grabbing",
    "e-resize",
    "n-resize",
    "ne-resize",
    "nw-resize",
    "s-resize",
    "se-resize",
    "sw-resize",
    "w-resize",
    "ew-resize",
    "ns-resize",
    "nesw-resize",
    "nwse-resize",
    "col-resize",
    "row-resize",
    "all-scroll",
    "zoom-in",
    "zoom-out",
];

impl Parse for Cursor {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        let mut images = vec![];
        while let Ok(url) = input.try_parse(|p| p.expect_url_or_string()) {
            let hotspot = input
                .try_parse(|p| Ok((parse_number(p, Range::All)?, parse_number(p, Range::All)?)))
                .ok();
            input.expect_comma()?;
            images.push((Box::<str>::from(url), hotspot));
        }
        let keyword = input.expect_ident()?.to_ascii_lowercase();
        let keyword = CURSOR_KEYWORDS
            .iter()
            .find(|k| **k == keyword)
            .ok_or(ParseError::Invalid)?;
        Ok(Cursor {
            images: images.into_boxed_slice(),
            keyword: (*keyword).into(),
        })
    }
}

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (url, hotspot) in &self.images {
            write_url(f, url)?;
            if let Some((x, y)) = hotspot {
                f.write_char(' ')?;
                write_number(f, *x)?;
                f.write_char(' ')?;
                write_number(f, *y)?;
            }
            f.write_str(", ")?;
        }
        f.write_str(&self.keyword)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AspectRatio {
    pub auto: bool,
    pub ratio: Option<(f32, f32)>,
}

impl Parse for AspectRatio {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        let mut auto = input.try_ident_matching("auto");
        let ratio = input
            .try_parse(|p| {
                let width = parse_number(p, Range::NonNegative)?;
                let height = if p.try_delim('/') {
                    parse_number(p, Range::NonNegative)?
                } else {
                    1.0
                };
                Ok((width, height))
            })
            .ok();
        if !auto {
            auto = input.try_ident_matching("auto");
        }
        if !auto && ratio.is_none() {
            return Err(ParseError::Invalid);
        }
        Ok(AspectRatio { auto, ratio })
    }
}

impl fmt::Display for AspectRatio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.auto {
            f.write_str("auto")?;
        }
        if let Some((w, h)) = self.ratio {
            if self.auto {
                f.write_char(' ')?;
            }
            write_number(f, w)?;
            f.write_str(" / ")?;
            write_number(f, h)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Start,
    End,
    Center,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PositionComponent {
    Side(Side, Option<LengthPercentage>),
    Length(LengthPercentage),
}

impl PositionComponent {
    pub const CENTER: PositionComponent = PositionComponent::Side(Side::Center, None);

    fn write(&self, f: &mut fmt::Formatter<'_>, horizontal: bool) -> fmt::Result {
        match self {
            PositionComponent::Length(lp) => lp.fmt(f),
            PositionComponent::Side(side, offset) => {
                f.write_str(match (side, horizontal) {
                    (Side::Center, _) => "center",
                    (Side::Start, true) => "left",
                    (Side::End, true) => "right",
                    (Side::Start, false) => "top",
                    (Side::End, false) => "bottom",
                })?;
                if let Some(offset) = offset {
                    write!(f, " {offset}")?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Axis {
    Horizontal,
    Vertical,
    Either,
}

fn position_keyword(input: &mut Parser<'_, '_>) -> ParseResult<(Side, Axis)> {
    let ident = input.expect_ident()?;
    Ok(match ident.to_ascii_lowercase().as_str() {
        "left" => (Side::Start, Axis::Horizontal),
        "right" => (Side::End, Axis::Horizontal),
        "top" => (Side::Start, Axis::Vertical),
        "bottom" => (Side::End, Axis::Vertical),
        "center" => (Side::Center, Axis::Either),
        _ => return Err(ParseError::Invalid),
    })
}

enum PositionPart {
    Keyword(Side, Axis),
    Length(LengthPercentage),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Position {
    pub x: PositionComponent,
    pub y: PositionComponent,
}

impl Position {
    pub const CENTER: Position = Position {
        x: PositionComponent::CENTER,
        y: PositionComponent::CENTER,
    };

    pub fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Position> {
        let mut parts = vec![];
        while parts.len() < 4 {
            if let Ok((side, axis)) = input.try_parse(position_keyword) {
                parts.push(PositionPart::Keyword(side, axis));
            } else if let Ok(lp) = input.try_parse(|p| LengthPercentage::parse(p, Range::All)) {
                parts.push(PositionPart::Length(lp));
            } else {
                break;
            }
        }
        Self::from_parts(parts)
    }

    fn from_parts(parts: Vec<PositionPart>) -> ParseResult<Position> {
        use PositionPart::{Keyword, Length};
        let center = || PositionComponent::CENTER;
        match parts.as_slice() {
            [Keyword(side, Axis::Vertical)] => Ok(Position {
                x: center(),
                y: PositionComponent::Side(*side, None),
            }),
            [Keyword(side, _)] => Ok(Position {
                x: PositionComponent::Side(*side, None),
                y: center(),
            }),
            [Length(lp)] => Ok(Position {
                x: PositionComponent::Length(lp.clone()),
                y: center(),
            }),
            [a, b] => {
                let horizontal_first = !matches!(a, Keyword(_, Axis::Vertical))
                    && !matches!(b, Keyword(_, Axis::Horizontal));
                let (x, y) = if horizontal_first { (a, b) } else { (b, a) };
                if matches!(x, Keyword(_, Axis::Vertical))
                    || matches!(y, Keyword(_, Axis::Horizontal))
                {
                    return Err(ParseError::Invalid);
                }
                if !horizontal_first && (matches!(a, Length(_)) || matches!(b, Length(_))) {
                    return Err(ParseError::Invalid);
                }
                let component = |part: &PositionPart| match part {
                    Keyword(side, _) => PositionComponent::Side(*side, None),
                    Length(lp) => PositionComponent::Length(lp.clone()),
                };
                Ok(Position {
                    x: component(x),
                    y: component(y),
                })
            }
            [_, _, _] | [_, _, _, _] => {
                let mut x = None;
                let mut y = None;
                let mut iter = parts.into_iter().peekable();
                while let Some(part) = iter.next() {
                    let Keyword(side, axis) = part else {
                        return Err(ParseError::Invalid);
                    };
                    let offset = match iter.peek() {
                        Some(Length(_)) => match iter.next() {
                            Some(Length(lp)) => Some(lp),
                            _ => None,
                        },
                        _ => None,
                    };
                    if side == Side::Center && offset.is_some() {
                        return Err(ParseError::Invalid);
                    }
                    let component = PositionComponent::Side(side, offset);
                    let horizontal = match axis {
                        Axis::Horizontal => true,
                        Axis::Vertical => false,
                        Axis::Either => {
                            x.is_none()
                                && !matches!(iter.peek(), Some(Keyword(_, Axis::Horizontal)))
                        }
                    };
                    let slot = if horizontal { &mut x } else { &mut y };
                    if slot.replace(component).is_some() {
                        return Err(ParseError::Invalid);
                    }
                }
                Ok(Position {
                    x: x.ok_or(ParseError::Invalid)?,
                    y: y.ok_or(ParseError::Invalid)?,
                })
            }
            _ => Err(ParseError::Invalid),
        }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.x.write(f, true)?;
        f.write_char(' ')?;
        self.y.write(f, false)
    }
}

fn parse_position_axis(
    input: &mut Parser<'_, '_>,
    horizontal: bool,
) -> ParseResult<PositionComponent> {
    if let Ok(lp) = input.try_parse(|p| LengthPercentage::parse(p, Range::All)) {
        return Ok(PositionComponent::Length(lp));
    }
    let (side, axis) = position_keyword(input)?;
    let wrong = if horizontal {
        Axis::Vertical
    } else {
        Axis::Horizontal
    };
    if axis == wrong {
        return Err(ParseError::Invalid);
    }
    let offset = if side == Side::Center {
        None
    } else {
        input
            .try_parse(|p| LengthPercentage::parse(p, Range::All))
            .ok()
    };
    Ok(PositionComponent::Side(side, offset))
}

macro_rules! position_list {
    ($name:ident, $horizontal:expr) => {
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name(pub Box<[PositionComponent]>);

        impl Parse for $name {
            fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
                input
                    .parse_comma_separated(|p| parse_position_axis(p, $horizontal))
                    .map(|list| $name(list.into_boxed_slice()))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                for (i, component) in self.0.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    component.write(f, $horizontal)?;
                }
                Ok(())
            }
        }
    };
}

position_list!(HorizontalPositionList, true);
position_list!(VerticalPositionList, false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepeatStyle {
    Repeat,
    Space,
    Round,
    NoRepeat,
}

impl RepeatStyle {
    fn from_ident(ident: &str) -> Option<RepeatStyle> {
        [
            ("repeat", RepeatStyle::Repeat),
            ("space", RepeatStyle::Space),
            ("round", RepeatStyle::Round),
            ("no-repeat", RepeatStyle::NoRepeat),
        ]
        .into_iter()
        .find(|(name, _)| ident.eq_ignore_ascii_case(name))
        .map(|(_, style)| style)
    }

    fn name(self) -> &'static str {
        match self {
            RepeatStyle::Repeat => "repeat",
            RepeatStyle::Space => "space",
            RepeatStyle::Round => "round",
            RepeatStyle::NoRepeat => "no-repeat",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackgroundRepeat(pub RepeatStyle, pub RepeatStyle);

impl BackgroundRepeat {
    pub(crate) fn parse_one(input: &mut Parser<'_, '_>) -> ParseResult<BackgroundRepeat> {
        let ident = input.expect_ident()?;
        if ident.eq_ignore_ascii_case("repeat-x") {
            return Ok(BackgroundRepeat(RepeatStyle::Repeat, RepeatStyle::NoRepeat));
        }
        if ident.eq_ignore_ascii_case("repeat-y") {
            return Ok(BackgroundRepeat(RepeatStyle::NoRepeat, RepeatStyle::Repeat));
        }
        let x = RepeatStyle::from_ident(&ident).ok_or(ParseError::Invalid)?;
        let y = input
            .try_parse(|p| RepeatStyle::from_ident(&p.expect_ident()?).ok_or(ParseError::Invalid))
            .unwrap_or(x);
        Ok(BackgroundRepeat(x, y))
    }
}

impl fmt::Display for BackgroundRepeat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.0, self.1) {
            (RepeatStyle::Repeat, RepeatStyle::NoRepeat) => f.write_str("repeat-x"),
            (RepeatStyle::NoRepeat, RepeatStyle::Repeat) => f.write_str("repeat-y"),
            (x, y) if x == y => f.write_str(x.name()),
            (x, y) => write!(f, "{} {}", x.name(), y.name()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BackgroundSize {
    Cover,
    Contain,
    Explicit(LengthPercentageOrAuto, LengthPercentageOrAuto),
}

impl BackgroundSize {
    pub(crate) fn parse_one(input: &mut Parser<'_, '_>) -> ParseResult<BackgroundSize> {
        if input.try_ident_matching("cover") {
            return Ok(BackgroundSize::Cover);
        }
        if input.try_ident_matching("contain") {
            return Ok(BackgroundSize::Contain);
        }
        let one = |p: &mut Parser<'_, '_>| {
            if p.try_ident_matching("auto") {
                Ok(LengthPercentageOrAuto::Auto)
            } else {
                LengthPercentage::parse(p, Range::NonNegative)
                    .map(LengthPercentageOrAuto::LengthPercentage)
            }
        };
        let width = one(input)?;
        let height = input.try_parse(one).unwrap_or(LengthPercentageOrAuto::Auto);
        Ok(BackgroundSize::Explicit(width, height))
    }
}

impl fmt::Display for BackgroundSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackgroundSize::Cover => f.write_str("cover"),
            BackgroundSize::Contain => f.write_str("contain"),
            BackgroundSize::Explicit(w, LengthPercentageOrAuto::Auto) => w.fmt(f),
            BackgroundSize::Explicit(w, h) => write!(f, "{w} {h}"),
        }
    }
}

macro_rules! comma_list {
    ($name:ident($item:ty), $parse:expr) => {
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name(pub Box<[$item]>);

        impl Parse for $name {
            fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
                let parse: fn(&mut Parser<'_, '_>) -> ParseResult<$item> = $parse;
                input
                    .parse_comma_separated(parse)
                    .map(|items| $name(items.into_boxed_slice()))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write_list(f, &self.0, ", ")
            }
        }
    };
}

comma_list!(
    BackgroundRepeatList(BackgroundRepeat),
    BackgroundRepeat::parse_one
);
comma_list!(
    BackgroundSizeList(BackgroundSize),
    BackgroundSize::parse_one
);
comma_list!(
    BackgroundAttachmentList(crate::properties::BackgroundAttachment),
    <crate::properties::BackgroundAttachment as Parse>::parse
);
comma_list!(
    BackgroundBoxList(crate::properties::BackgroundBox),
    <crate::properties::BackgroundBox as Parse>::parse
);
comma_list!(ImageList(Image), <Image as Parse>::parse);

#[derive(Clone, Debug, PartialEq)]
pub struct ColorStop {
    pub color: Option<Color>,
    pub position: Option<LengthPercentage>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GradientKind {
    Linear(LinearDirection),
    Radial {
        circle: bool,
        size: RadialSize,
        position: Position,
    },
    Conic {
        from: f32,
        position: Position,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum LinearDirection {
    Angle(f32),
    Side(Side, Side),
}

#[derive(Clone, Debug, PartialEq)]
pub enum RadialSize {
    ClosestSide,
    ClosestCorner,
    FarthestSide,
    FarthestCorner,
    Explicit(LengthPercentage, Option<LengthPercentage>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub kind: GradientKind,
    pub repeating: bool,
    pub stops: Box<[ColorStop]>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Image {
    None,
    Url(Box<str>),
    Gradient(Box<Gradient>),
}

fn parse_linear_direction(input: &mut Parser<'_, '_>) -> ParseResult<LinearDirection> {
    if let Ok(angle) = input.try_parse(|p| parse_angle(p, true)) {
        return Ok(LinearDirection::Angle(angle));
    }
    input.expect_ident_matching("to")?;
    let mut x = None;
    let mut y = None;
    for _ in 0..2 {
        let Ok((side, axis)) = input.try_parse(position_keyword) else {
            break;
        };
        let slot = match axis {
            Axis::Horizontal => &mut x,
            Axis::Vertical => &mut y,
            Axis::Either => return Err(ParseError::Invalid),
        };
        if slot.replace(side).is_some() {
            return Err(ParseError::Invalid);
        }
    }
    match (x, y) {
        (None, None) => Err(ParseError::Invalid),
        (x, y) => Ok(LinearDirection::Side(
            x.unwrap_or(Side::Center),
            y.unwrap_or(Side::Center),
        )),
    }
}

fn parse_radial_shape(input: &mut Parser<'_, '_>) -> ParseResult<(bool, RadialSize)> {
    let mut circle = None;
    let mut size = None;
    for _ in 0..2 {
        if circle.is_none() {
            if input.try_ident_matching("circle") {
                circle = Some(true);
                continue;
            }
            if input.try_ident_matching("ellipse") {
                circle = Some(false);
                continue;
            }
        }
        if size.is_none() {
            let keyword = [
                ("closest-side", RadialSize::ClosestSide),
                ("closest-corner", RadialSize::ClosestCorner),
                ("farthest-side", RadialSize::FarthestSide),
                ("farthest-corner", RadialSize::FarthestCorner),
            ]
            .into_iter()
            .find(|(name, _)| input.try_ident_matching(name));
            if let Some((_, keyword)) = keyword {
                size = Some(keyword);
                continue;
            }
            if let Ok(first) = input.try_parse(|p| LengthPercentage::parse(p, Range::NonNegative)) {
                let second = input
                    .try_parse(|p| LengthPercentage::parse(p, Range::NonNegative))
                    .ok();
                size = Some(RadialSize::Explicit(first, second));
                continue;
            }
        }
        break;
    }
    if circle.is_none() && size.is_none() {
        return Err(ParseError::Invalid);
    }
    let size = size.unwrap_or(RadialSize::FarthestCorner);
    let circle = match (&size, circle) {
        (_, Some(circle)) => circle,
        (RadialSize::Explicit(_, None), None) => true,
        _ => false,
    };
    Ok((circle, size))
}

fn parse_stops(input: &mut Parser<'_, '_>, angular: bool) -> ParseResult<Box<[ColorStop]>> {
    let position = |p: &mut Parser<'_, '_>| {
        if angular {
            p.try_parse(|p| parse_angle(p, true).map(|deg| LengthPercentage::Percentage(deg / 3.6)))
                .or_else(|_| parse_percentage(p, Range::All).map(LengthPercentage::Percentage))
        } else {
            LengthPercentage::parse(p, Range::All)
        }
    };
    let mut stops = vec![];
    loop {
        let color = input.try_parse(Color::parse).ok();
        let first = input.try_parse(position).ok();
        match (color, first) {
            (Some(color), first) => {
                let second = first.as_ref().and_then(|_| input.try_parse(position).ok());
                stops.push(ColorStop {
                    color: Some(color.clone()),
                    position: first,
                });
                if second.is_some() {
                    stops.push(ColorStop {
                        color: Some(color),
                        position: second,
                    });
                }
            }
            (None, Some(hint)) => {
                if stops.last().is_none_or(|s: &ColorStop| s.color.is_none()) {
                    return Err(ParseError::Invalid);
                }
                stops.push(ColorStop {
                    color: None,
                    position: Some(hint),
                });
            }
            (None, None) => return Err(ParseError::Invalid),
        }
        if input.is_exhausted() {
            break;
        }
        input.expect_comma()?;
    }
    if stops.len() < 2 || stops.last().is_some_and(|s| s.color.is_none()) {
        return Err(ParseError::Invalid);
    }
    Ok(stops.into_boxed_slice())
}

fn parse_gradient(name: &str, input: &mut Parser<'_, '_>) -> ParseResult<Gradient> {
    let (base, repeating) = match name.strip_prefix("repeating-") {
        Some(base) => (base, true),
        None => (name, false),
    };
    let base = base.strip_prefix("-webkit-").unwrap_or(base);
    let kind = match base {
        "linear-gradient" => {
            let direction = input.try_parse(|p| {
                let direction = parse_linear_direction(p)?;
                p.expect_comma()?;
                Ok(direction)
            });
            GradientKind::Linear(direction.unwrap_or(LinearDirection::Angle(180.0)))
        }
        "radial-gradient" => {
            let shape = input.try_parse(parse_radial_shape).ok();
            let position = if input.try_ident_matching("at") {
                Position::parse(input)?
            } else {
                Position::CENTER
            };
            if shape.is_some() || position != Position::CENTER {
                input.expect_comma()?;
            }
            let (circle, size) = shape.unwrap_or((false, RadialSize::FarthestCorner));
            GradientKind::Radial {
                circle,
                size,
                position,
            }
        }
        "conic-gradient" => {
            let from = if input.try_ident_matching("from") {
                Some(parse_angle(input, true)?)
            } else {
                None
            };
            let position = if input.try_ident_matching("at") {
                Some(Position::parse(input)?)
            } else {
                None
            };
            if from.is_some() || position.is_some() {
                input.expect_comma()?;
            }
            GradientKind::Conic {
                from: from.unwrap_or(0.0),
                position: position.unwrap_or(Position::CENTER),
            }
        }
        _ => return Err(ParseError::Invalid),
    };
    let stops = parse_stops(input, matches!(kind, GradientKind::Conic { .. }))?;
    Ok(Gradient {
        kind,
        repeating,
        stops,
    })
}

impl Parse for Image {
    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {
        if input.try_ident_matching("none") {
            return Ok(Image::None);
        }
        if let Ok(url) = input.try_parse(|p| match p.next_token()? {
            Token::Url(url) => Ok(url),
            _ => Err(ParseError::Invalid),
        }) {
            return Ok(Image::Url(url.into()));
        }
        let name = input.expect_function()?.to_ascii_lowercase();
        input.parse_nested_block(|p| {
            let image = if name == "url" {
                Image::Url(p.expect_string()?.into())
            } else {
                Image::Gradient(Box::new(parse_gradient(&name, p)?))
            };
            p.expect_exhausted()?;
            Ok(image)
        })
    }
}

impl fmt::Display for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Image::None => f.write_str("none"),
            Image::Url(url) => write_url(f, url),
            Image::Gradient(gradient) => gradient.fmt(f),
        }
    }
}

impl fmt::Display for Gradient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.repeating {
            f.write_str("repeating-")?;
        }
        match &self.kind {
            GradientKind::Linear(direction) => {
                f.write_str("linear-gradient(")?;
                match direction {
                    LinearDirection::Angle(angle) => {
                        write_number(f, *angle)?;
                        f.write_str("deg, ")?;
                    }
                    LinearDirection::Side(x, y) => {
                        f.write_str("to")?;
                        let x = PositionComponent::Side(*x, None);
                        let y = PositionComponent::Side(*y, None);
                        if x != PositionComponent::CENTER {
                            f.write_char(' ')?;
                            x.write(f, true)?;
                        }
                        if y != PositionComponent::CENTER {
                            f.write_char(' ')?;
                            y.write(f, false)?;
                        }
                        f.write_str(", ")?;
                    }
                }
            }
            GradientKind::Radial {
                circle,
                size,
                position,
            } => {
                f.write_str("radial-gradient(")?;
                f.write_str(if *circle { "circle " } else { "ellipse " })?;
                match size {
                    RadialSize::ClosestSide => f.write_str("closest-side")?,
                    RadialSize::ClosestCorner => f.write_str("closest-corner")?,
                    RadialSize::FarthestSide => f.write_str("farthest-side")?,
                    RadialSize::FarthestCorner => f.write_str("farthest-corner")?,
                    RadialSize::Explicit(a, None) => a.fmt(f)?,
                    RadialSize::Explicit(a, Some(b)) => write!(f, "{a} {b}")?,
                }
                write!(f, " at {position}, ")?;
            }
            GradientKind::Conic { from, position } => {
                f.write_str("conic-gradient(from ")?;
                write_number(f, *from)?;
                write!(f, "deg at {position}, ")?;
            }
        }
        for (i, stop) in self.stops.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            match (&stop.color, &stop.position) {
                (Some(color), Some(position)) => write!(f, "{color} {position}")?,
                (Some(color), None) => color.fmt(f)?,
                (None, Some(position)) => position.fmt(f)?,
                (None, None) => {}
            }
        }
        f.write_char(')')
    }
}
