use crate::parser::{ParseError, ParseResult, Parser};
use crate::tokenizer::Token;
use crate::values::{Length, Range, parse_number, parse_resolution};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaType {
    All,
    Screen,
    Print,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    Lt,
    Le,
    Eq,
    Ge,
    Gt,
}

impl Comparison {
    fn flipped(self) -> Comparison {
        match self {
            Comparison::Lt => Comparison::Gt,
            Comparison::Le => Comparison::Ge,
            Comparison::Eq => Comparison::Eq,
            Comparison::Ge => Comparison::Le,
            Comparison::Gt => Comparison::Lt,
        }
    }

    fn holds(self, actual: f32, expected: f32) -> bool {
        match self {
            Comparison::Lt => actual < expected,
            Comparison::Le => actual <= expected,
            Comparison::Eq => (actual - expected).abs() < 1e-6,
            Comparison::Ge => actual >= expected,
            Comparison::Gt => actual > expected,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FeatureValue {
    Length(Length),
    Number(f32),
    Ratio(f32, f32),
    Resolution(f32),
    Ident(Box<str>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureName {
    Width,
    Height,
    AspectRatio,
    Orientation,
    Resolution,
    DeviceWidth,
    DeviceHeight,
    DeviceAspectRatio,
    PrefersColorScheme,
    PrefersReducedMotion,
    PrefersContrast,
    PrefersReducedTransparency,
    ForcedColors,
    Hover,
    AnyHover,
    Pointer,
    AnyPointer,
    Color,
    ColorGamut,
    Monochrome,
    DisplayMode,
    DynamicRange,
    Scripting,
    Update,
    Grid,
    InvertedColors,
}

const FEATURES: &[(&str, FeatureName, bool)] = &[
    ("width", FeatureName::Width, true),
    ("height", FeatureName::Height, true),
    ("aspect-ratio", FeatureName::AspectRatio, true),
    ("orientation", FeatureName::Orientation, false),
    ("resolution", FeatureName::Resolution, true),
    ("-webkit-device-pixel-ratio", FeatureName::Resolution, true),
    ("device-width", FeatureName::DeviceWidth, true),
    ("device-height", FeatureName::DeviceHeight, true),
    ("device-aspect-ratio", FeatureName::DeviceAspectRatio, true),
    (
        "prefers-color-scheme",
        FeatureName::PrefersColorScheme,
        false,
    ),
    (
        "prefers-reduced-motion",
        FeatureName::PrefersReducedMotion,
        false,
    ),
    ("prefers-contrast", FeatureName::PrefersContrast, false),
    (
        "prefers-reduced-transparency",
        FeatureName::PrefersReducedTransparency,
        false,
    ),
    ("forced-colors", FeatureName::ForcedColors, false),
    ("hover", FeatureName::Hover, false),
    ("any-hover", FeatureName::AnyHover, false),
    ("pointer", FeatureName::Pointer, false),
    ("any-pointer", FeatureName::AnyPointer, false),
    ("color", FeatureName::Color, true),
    ("color-gamut", FeatureName::ColorGamut, false),
    ("monochrome", FeatureName::Monochrome, true),
    ("display-mode", FeatureName::DisplayMode, false),
    ("dynamic-range", FeatureName::DynamicRange, false),
    ("scripting", FeatureName::Scripting, false),
    ("update", FeatureName::Update, false),
    ("grid", FeatureName::Grid, false),
    ("inverted-colors", FeatureName::InvertedColors, false),
];

#[derive(Clone, Debug, PartialEq)]
pub struct MediaFeature {
    pub name: FeatureName,
    pub constraints: Box<[(Comparison, FeatureValue)]>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MediaCondition {
    Feature(MediaFeature),
    Not(Box<MediaCondition>),
    And(Box<[MediaCondition]>),
    Or(Box<[MediaCondition]>),
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaQuery {
    pub negated: bool,
    pub media_type: MediaType,
    pub condition: Option<MediaCondition>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaList(pub Box<[MediaQuery]>);

#[derive(Clone, Debug, PartialEq)]
pub struct MediaEnvironment {
    pub print: bool,
    pub width: f32,
    pub height: f32,
    pub device_width: f32,
    pub device_height: f32,
    pub resolution: f32,
    pub font_size: f32,
    pub dark: bool,
    pub reduced_motion: bool,
    pub hover: bool,
    pub fine_pointer: bool,
    pub wide_gamut: bool,
}

const NOT_ALL: MediaQuery = MediaQuery {
    negated: true,
    media_type: MediaType::All,
    condition: None,
};

impl MediaList {
    pub fn parse(input: &mut Parser<'_, '_>) -> MediaList {
        input.skip_whitespace();
        if input.is_exhausted() {
            return MediaList::default();
        }
        let mut queries = vec![];
        loop {
            let query = input.parse_until_before(crate::parser::Delimiters::COMMA, |p| {
                let query = parse_query(p)?;
                p.expect_exhausted()?;
                Ok(query)
            });
            queries.push(query.unwrap_or(NOT_ALL));
            if input.next_token().is_err() {
                break;
            }
        }
        MediaList(queries.into_boxed_slice())
    }

    pub fn matches(&self, env: &MediaEnvironment) -> bool {
        self.0.is_empty() || self.0.iter().any(|query| query.matches(env))
    }
}

impl MediaQuery {
    pub fn matches(&self, env: &MediaEnvironment) -> bool {
        let type_matches = match self.media_type {
            MediaType::All => true,
            MediaType::Screen => !env.print,
            MediaType::Print => env.print,
            MediaType::Unknown => false,
        };
        let result = type_matches
            && self
                .condition
                .as_ref()
                .is_none_or(|condition| condition.matches(env));
        result != self.negated
    }
}

fn parse_query(input: &mut Parser<'_, '_>) -> ParseResult<MediaQuery> {
    if let Ok(condition) = input.try_parse(|p| parse_condition(p, true)) {
        return Ok(MediaQuery {
            negated: false,
            media_type: MediaType::All,
            condition: Some(condition),
        });
    }
    let mut negated = false;
    let mut ident = input.expect_ident()?;
    if ident.eq_ignore_ascii_case("not") {
        negated = true;
        ident = input.expect_ident()?;
    } else if ident.eq_ignore_ascii_case("only") {
        ident = input.expect_ident()?;
    }
    let lower = ident.to_ascii_lowercase();
    let media_type = match lower.as_str() {
        "all" => MediaType::All,
        "screen" => MediaType::Screen,
        "print" => MediaType::Print,
        "not" | "and" | "or" | "only" | "layer" => return Err(ParseError::Invalid),
        _ => MediaType::Unknown,
    };
    let condition = if input.try_ident_matching("and") {
        Some(parse_condition(input, false)?)
    } else {
        None
    };
    Ok(MediaQuery {
        negated,
        media_type,
        condition,
    })
}

pub(crate) fn parse_condition(
    input: &mut Parser<'_, '_>,
    allow_or: bool,
) -> ParseResult<MediaCondition> {
    if input.try_ident_matching("not") {
        return Ok(MediaCondition::Not(Box::new(parse_in_parens(input)?)));
    }
    let first = parse_in_parens(input)?;
    let mut items = vec![first];
    let mut is_or = None;
    loop {
        let state = input.state();
        let Ok(ident) = input.expect_ident() else {
            input.reset(&state);
            break;
        };
        let or = if ident.eq_ignore_ascii_case("and") {
            false
        } else if ident.eq_ignore_ascii_case("or") && allow_or {
            true
        } else {
            input.reset(&state);
            break;
        };
        if is_or.is_some_and(|previous| previous != or) {
            return Err(ParseError::Invalid);
        }
        is_or = Some(or);
        items.push(parse_in_parens(input)?);
    }
    Ok(match (items.len(), is_or) {
        (1, _) => items.pop().unwrap_or(MediaCondition::Unknown),
        (_, Some(true)) => MediaCondition::Or(items.into_boxed_slice()),
        _ => MediaCondition::And(items.into_boxed_slice()),
    })
}

fn parse_in_parens(input: &mut Parser<'_, '_>) -> ParseResult<MediaCondition> {
    match input.next_token()? {
        Token::OpenParen => input.parse_nested_block(|p| {
            if let Ok(condition) = p.try_parse(|p| {
                let condition = parse_condition(p, true)?;
                p.expect_exhausted()?;
                Ok(condition)
            }) {
                return Ok(condition);
            }
            if let Ok(feature) = p.try_parse(|p| {
                let feature = parse_feature(p)?;
                p.expect_exhausted()?;
                Ok(feature)
            }) {
                return Ok(feature);
            }
            while p.next_including_whitespace().is_ok() {}
            Ok(MediaCondition::Unknown)
        }),
        Token::Function(_) => {
            input.parse_nested_block(|p| {
                while p.next_including_whitespace().is_ok() {}
                Ok(())
            })?;
            Ok(MediaCondition::Unknown)
        }
        _ => Err(ParseError::Invalid),
    }
}

fn parse_value(input: &mut Parser<'_, '_>) -> ParseResult<FeatureValue> {
    if let Ok(numerator) = input.try_parse(|p| parse_number(p, Range::All)) {
        if input.try_delim('/') {
            return Ok(FeatureValue::Ratio(
                numerator,
                parse_number(input, Range::All)?,
            ));
        }
        return Ok(FeatureValue::Number(numerator));
    }
    if let Ok(dppx) = input.try_parse(parse_resolution) {
        return Ok(FeatureValue::Resolution(dppx));
    }
    if let Ok(crate::values::LengthPercentage::Length(length)) =
        input.try_parse(|p| crate::values::LengthPercentage::parse_length(p, Range::All))
    {
        return Ok(FeatureValue::Length(length));
    }
    Ok(FeatureValue::Ident(
        input.expect_ident()?.to_ascii_lowercase().into(),
    ))
}

fn parse_comparison(input: &mut Parser<'_, '_>) -> ParseResult<Comparison> {
    let first = match input.next_token()? {
        Token::Delim(c @ ('<' | '>' | '=')) => c,
        _ => return Err(ParseError::Invalid),
    };
    if first == '=' {
        return Ok(Comparison::Eq);
    }
    let equal = input
        .try_parse(|p| match p.next_including_whitespace()? {
            Token::Delim('=') => Ok(()),
            _ => Err(ParseError::Invalid),
        })
        .is_ok();
    Ok(match (first, equal) {
        ('<', false) => Comparison::Lt,
        ('<', true) => Comparison::Le,
        ('>', false) => Comparison::Gt,
        _ => Comparison::Ge,
    })
}

fn feature_name(name: &str) -> ParseResult<(FeatureName, bool)> {
    FEATURES
        .iter()
        .find(|(n, _, _)| name.eq_ignore_ascii_case(n))
        .map(|(_, feature, range)| (*feature, *range))
        .ok_or(ParseError::Invalid)
}

fn parse_feature(input: &mut Parser<'_, '_>) -> ParseResult<MediaCondition> {
    if let Ok(name) = input.try_parse(|p| p.expect_ident()) {
        let lower = name.to_ascii_lowercase();
        let (prefix, base) = match (lower.strip_prefix("min-"), lower.strip_prefix("max-")) {
            (Some(base), _) => (Some(Comparison::Ge), base),
            (_, Some(base)) => (Some(Comparison::Le), base),
            _ => (None, lower.as_str()),
        };
        let base = base
            .replace("-webkit-min-", "-webkit-")
            .replace("-webkit-max-", "-webkit-");
        let (feature, range) = match feature_name(&base) {
            Ok(found) => found,
            Err(_) => {
                let webkit = lower
                    .strip_prefix("-webkit-min-")
                    .map(|b| (Comparison::Ge, b))
                    .or_else(|| {
                        lower
                            .strip_prefix("-webkit-max-")
                            .map(|b| (Comparison::Le, b))
                    });
                match webkit {
                    Some((comparison, b)) => {
                        let (feature, _) = feature_name(&format!("-webkit-{b}"))?;
                        input.expect_token(Token::Colon)?;
                        let value = parse_value(input)?;
                        return Ok(MediaCondition::Feature(MediaFeature {
                            name: feature,
                            constraints: Box::new([(comparison, value)]),
                        }));
                    }
                    None => return Ok(MediaCondition::Unknown),
                }
            }
        };
        if prefix.is_some() && !range {
            return Err(ParseError::Invalid);
        }
        if input.is_exhausted() {
            if prefix.is_some() {
                return Err(ParseError::Invalid);
            }
            return Ok(MediaCondition::Feature(MediaFeature {
                name: feature,
                constraints: Box::new([]),
            }));
        }
        if input.try_parse(|p| p.expect_token(Token::Colon)).is_ok() {
            let value = parse_value(input)?;
            return Ok(MediaCondition::Feature(MediaFeature {
                name: feature,
                constraints: Box::new([(prefix.unwrap_or(Comparison::Eq), value)]),
            }));
        }
        if prefix.is_some() || !range {
            return Err(ParseError::Invalid);
        }
        let comparison = parse_comparison(input)?;
        let value = parse_value(input)?;
        return Ok(MediaCondition::Feature(MediaFeature {
            name: feature,
            constraints: Box::new([(comparison, value)]),
        }));
    }
    let low = parse_value(input)?;
    let first = parse_comparison(input)?;
    let (feature, range) = feature_name(&input.expect_ident()?)?;
    if !range {
        return Err(ParseError::Invalid);
    }
    let mut constraints = vec![(first.flipped(), low)];
    if let Ok(second) = input.try_parse(parse_comparison) {
        let directions = |c: Comparison| matches!(c, Comparison::Lt | Comparison::Le);
        if directions(first) != directions(second) || first == Comparison::Eq {
            return Err(ParseError::Invalid);
        }
        constraints.push((second, parse_value(input)?));
    }
    Ok(MediaCondition::Feature(MediaFeature {
        name: feature,
        constraints: constraints.into_boxed_slice(),
    }))
}

impl MediaCondition {
    pub fn matches(&self, env: &MediaEnvironment) -> bool {
        match self {
            MediaCondition::Feature(feature) => feature.matches(env),
            MediaCondition::Not(inner) => !inner.matches(env),
            MediaCondition::And(items) => items.iter().all(|c| c.matches(env)),
            MediaCondition::Or(items) => items.iter().any(|c| c.matches(env)),
            MediaCondition::Unknown => false,
        }
    }
}

impl MediaFeature {
    fn matches(&self, env: &MediaEnvironment) -> bool {
        let numeric = |value: &FeatureValue| -> Option<f32> {
            match value {
                FeatureValue::Number(n) => Some(*n),
                FeatureValue::Ratio(a, b) => Some(a / b),
                FeatureValue::Resolution(r) => Some(*r),
                FeatureValue::Length(length) => length.to_px().or(match length.unit {
                    crate::values::LengthUnit::Em | crate::values::LengthUnit::Rem => {
                        Some(length.value * env.font_size)
                    }
                    _ => None,
                }),
                FeatureValue::Ident(_) => None,
            }
        };
        let range = |actual: f32| {
            self.constraints.iter().all(|(comparison, value)| {
                numeric(value).is_some_and(|expected| comparison.holds(actual, expected))
            })
        };
        let keyword = |actual: &str, enabled: bool| match self.constraints.first() {
            None => enabled,
            Some((_, FeatureValue::Ident(value))) => &**value == actual,
            Some(_) => false,
        };
        match self.name {
            FeatureName::Width => self.boolean_or(env.width != 0.0, range(env.width)),
            FeatureName::Height => self.boolean_or(env.height != 0.0, range(env.height)),
            FeatureName::AspectRatio => {
                self.boolean_or(true, range(env.width / env.height.max(1.0)))
            }
            FeatureName::DeviceWidth => self.boolean_or(true, range(env.device_width)),
            FeatureName::DeviceHeight => self.boolean_or(true, range(env.device_height)),
            FeatureName::DeviceAspectRatio => {
                self.boolean_or(true, range(env.device_width / env.device_height.max(1.0)))
            }
            FeatureName::Resolution => self.boolean_or(true, range(env.resolution)),
            FeatureName::Color => self.boolean_or(true, range(8.0)),
            FeatureName::Monochrome => self.boolean_or(false, range(0.0)),
            FeatureName::Orientation => keyword(
                if env.height >= env.width {
                    "portrait"
                } else {
                    "landscape"
                },
                true,
            ),
            FeatureName::PrefersColorScheme => {
                keyword(if env.dark { "dark" } else { "light" }, true)
            }
            FeatureName::PrefersReducedMotion => keyword(
                if env.reduced_motion {
                    "reduce"
                } else {
                    "no-preference"
                },
                env.reduced_motion,
            ),
            FeatureName::PrefersContrast => keyword("no-preference", false),
            FeatureName::PrefersReducedTransparency => keyword("no-preference", false),
            FeatureName::ForcedColors => keyword("none", false),
            FeatureName::InvertedColors => keyword("none", false),
            FeatureName::Hover | FeatureName::AnyHover => {
                keyword(if env.hover { "hover" } else { "none" }, env.hover)
            }
            FeatureName::Pointer | FeatureName::AnyPointer => {
                keyword(if env.fine_pointer { "fine" } else { "coarse" }, true)
            }
            FeatureName::ColorGamut => match self.constraints.first() {
                None => true,
                Some((_, FeatureValue::Ident(value))) => {
                    &**value == "srgb" || (env.wide_gamut && &**value == "p3")
                }
                Some(_) => false,
            },
            FeatureName::DisplayMode => keyword("browser", true),
            FeatureName::DynamicRange => keyword("standard", true),
            FeatureName::Scripting => keyword("none", false),
            FeatureName::Update => keyword(if env.print { "none" } else { "fast" }, !env.print),
            FeatureName::Grid => self.boolean_or(false, range(0.0)),
        }
    }

    fn boolean_or(&self, boolean: bool, range: bool) -> bool {
        if self.constraints.is_empty() {
            boolean
        } else {
            range
        }
    }
}
