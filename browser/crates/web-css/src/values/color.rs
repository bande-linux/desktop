use std::fmt::{self, Write};

use crate::parser::{ParseError, ParseResult, Parser};
use crate::tokenizer::Token;
use crate::values::angle_to_degrees;
use crate::values::calc::{CalcLeaf, CalcParser, is_math_function};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const TRANSPARENT: Rgba = Rgba::new(0.0, 0.0, 0.0, 0.0);
    pub const BLACK: Rgba = Rgba::new(0.0, 0.0, 0.0, 1.0);

    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Rgba {
        Rgba { r, g, b, a }
    }

    const fn from_bytes([r, g, b]: [u8; 3]) -> Rgba {
        Rgba::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSpace {
    Srgb,
    SrgbLinear,
    DisplayP3,
    A98Rgb,
    ProphotoRgb,
    Rec2020,
    XyzD50,
    XyzD65,
    Lab,
    Lch,
    Oklab,
    Oklch,
    Hsl,
    Hwb,
}

impl ColorSpace {
    fn from_name(name: &str) -> Option<ColorSpace> {
        Some(match name.to_ascii_lowercase().as_str() {
            "srgb" => ColorSpace::Srgb,
            "srgb-linear" => ColorSpace::SrgbLinear,
            "display-p3" => ColorSpace::DisplayP3,
            "a98-rgb" => ColorSpace::A98Rgb,
            "prophoto-rgb" => ColorSpace::ProphotoRgb,
            "rec2020" => ColorSpace::Rec2020,
            "xyz" | "xyz-d65" => ColorSpace::XyzD65,
            "xyz-d50" => ColorSpace::XyzD50,
            "lab" => ColorSpace::Lab,
            "lch" => ColorSpace::Lch,
            "oklab" => ColorSpace::Oklab,
            "oklch" => ColorSpace::Oklch,
            "hsl" => ColorSpace::Hsl,
            "hwb" => ColorSpace::Hwb,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            ColorSpace::Srgb => "srgb",
            ColorSpace::SrgbLinear => "srgb-linear",
            ColorSpace::DisplayP3 => "display-p3",
            ColorSpace::A98Rgb => "a98-rgb",
            ColorSpace::ProphotoRgb => "prophoto-rgb",
            ColorSpace::Rec2020 => "rec2020",
            ColorSpace::XyzD50 => "xyz-d50",
            ColorSpace::XyzD65 => "xyz-d65",
            ColorSpace::Lab => "lab",
            ColorSpace::Lch => "lch",
            ColorSpace::Oklab => "oklab",
            ColorSpace::Oklch => "oklch",
            ColorSpace::Hsl => "hsl",
            ColorSpace::Hwb => "hwb",
        }
    }

    fn hue_index(self) -> Option<usize> {
        match self {
            ColorSpace::Lch | ColorSpace::Oklch => Some(2),
            ColorSpace::Hsl | ColorSpace::Hwb => Some(0),
            _ => None,
        }
    }

    fn is_predefined(self) -> bool {
        !matches!(
            self,
            ColorSpace::Lab
                | ColorSpace::Lch
                | ColorSpace::Oklab
                | ColorSpace::Oklch
                | ColorSpace::Hsl
                | ColorSpace::Hwb
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AbsoluteColor {
    pub space: ColorSpace,
    pub components: [f64; 3],
    pub alpha: f64,
    pub missing: u8,
    pub legacy: bool,
}

const ALPHA_MISSING: u8 = 1 << 3;

impl AbsoluteColor {
    pub fn srgb_legacy(rgba: Rgba) -> AbsoluteColor {
        AbsoluteColor {
            space: ColorSpace::Srgb,
            components: [rgba.r, rgba.g, rgba.b].map(f64::from),
            alpha: rgba.a.into(),
            missing: 0,
            legacy: true,
        }
    }

    pub fn to_rgba(&self) -> Rgba {
        let [r, g, b] = self.to_space(ColorSpace::Srgb).components;
        Rgba::new(
            r.clamp(0.0, 1.0) as f32,
            g.clamp(0.0, 1.0) as f32,
            b.clamp(0.0, 1.0) as f32,
            self.alpha.clamp(0.0, 1.0) as f32,
        )
    }

    pub fn to_space(&self, space: ColorSpace) -> AbsoluteColor {
        let components = if self.space == space {
            self.components
        } else {
            from_xyz_d65(space, to_xyz_d65(self.space, self.resolved_components()))
        };
        AbsoluteColor {
            space,
            components,
            alpha: self.alpha,
            missing: self.missing & ALPHA_MISSING,
            legacy: false,
        }
    }

    fn resolved_components(&self) -> [f64; 3] {
        let mut c = self.components;
        for (i, value) in c.iter_mut().enumerate() {
            if self.missing & (1 << i) != 0 {
                *value = 0.0;
            }
        }
        c
    }
}

macro_rules! system_colors {
    ($($variant:ident = $text:literal => [$r:expr, $g:expr, $b:expr],)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum SystemColor { $($variant,)* }

        impl SystemColor {
            fn from_name(name: &str) -> Option<SystemColor> {
                $(if name.eq_ignore_ascii_case($text) { return Some(SystemColor::$variant); })*
                None
            }

            pub fn name(self) -> &'static str {
                match self { $(SystemColor::$variant => $text,)* }
            }

            pub fn light_rgba(self) -> Rgba {
                match self { $(SystemColor::$variant => Rgba::from_bytes([$r, $g, $b]),)* }
            }
        }
    };
}

system_colors! {
    AccentColor = "AccentColor" => [0, 117, 255],
    AccentColorText = "AccentColorText" => [255, 255, 255],
    ActiveText = "ActiveText" => [255, 0, 0],
    ButtonBorder = "ButtonBorder" => [118, 118, 118],
    ButtonFace = "ButtonFace" => [239, 239, 239],
    ButtonText = "ButtonText" => [0, 0, 0],
    Canvas = "Canvas" => [255, 255, 255],
    CanvasText = "CanvasText" => [0, 0, 0],
    Field = "Field" => [255, 255, 255],
    FieldText = "FieldText" => [0, 0, 0],
    GrayText = "GrayText" => [109, 109, 109],
    Highlight = "Highlight" => [181, 213, 255],
    HighlightText = "HighlightText" => [0, 0, 0],
    LinkText = "LinkText" => [0, 0, 238],
    Mark = "Mark" => [255, 255, 0],
    MarkText = "MarkText" => [0, 0, 0],
    SelectedItem = "SelectedItem" => [0, 117, 255],
    SelectedItemText = "SelectedItemText" => [255, 255, 255],
    VisitedText = "VisitedText" => [85, 26, 139],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HueMethod {
    Shorter,
    Longer,
    Increasing,
    Decreasing,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColorMix {
    pub space: ColorSpace,
    pub hue: HueMethod,
    pub first: Color,
    pub first_percentage: Option<f64>,
    pub second: Color,
    pub second_percentage: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProfiledColor {
    pub profile: Box<str>,
    pub components: Box<[f64]>,
    pub alpha: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Color {
    CurrentColor,
    Absolute(AbsoluteColor),
    System(SystemColor),
    LightDark(Box<[Color; 2]>),
    Mix(Box<ColorMix>),
    DeviceCmyk(Box<[f64; 5]>),
    Profiled(Box<ProfiledColor>),
}

impl Color {
    pub const TRANSPARENT: Color = Color::Absolute(AbsoluteColor {
        space: ColorSpace::Srgb,
        components: [0.0; 3],
        alpha: 0.0,
        missing: 0,
        legacy: true,
    });

    pub fn rgba(rgba: Rgba) -> Color {
        Color::Absolute(AbsoluteColor::srgb_legacy(rgba))
    }

    pub fn resolve(&self, current: Rgba, dark: bool) -> Rgba {
        match self {
            Color::CurrentColor => current,
            Color::Absolute(color) => color.to_rgba(),
            Color::System(system) => system.light_rgba(),
            Color::LightDark(pair) => pair[usize::from(dark)].resolve(current, dark),
            Color::Mix(mix) => {
                let first = AbsoluteColor::srgb_legacy(mix.first.resolve(current, dark));
                let second = AbsoluteColor::srgb_legacy(mix.second.resolve(current, dark));
                mix_absolute(mix, &first, &second).map_or(Rgba::TRANSPARENT, |c| c.to_rgba())
            }
            Color::DeviceCmyk(cmyk) => {
                let [c, m, y, k, a] = **cmyk;
                Rgba::new(
                    (1.0 - (c * (1.0 - k) + k).min(1.0)) as f32,
                    (1.0 - (m * (1.0 - k) + k).min(1.0)) as f32,
                    (1.0 - (y * (1.0 - k) + k).min(1.0)) as f32,
                    a as f32,
                )
            }
            Color::Profiled(_) => Rgba::TRANSPARENT,
        }
    }

    pub fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Color> {
        match input.next_token()? {
            Token::IdHash(hex) | Token::Hash(hex) => parse_hex(&hex).map(byte_color),
            Token::Ident(name) => parse_named(&name),
            Token::Function(name) => {
                let name = name.to_ascii_lowercase();
                input.parse_nested_block(|p| {
                    let color = parse_function(&name, p)?;
                    p.expect_exhausted()?;
                    Ok(color)
                })
            }
            _ => Err(ParseError::Invalid),
        }
    }
}

include!("named_colors.rs");

fn parse_named(name: &str) -> ParseResult<Color> {
    let lower = name.to_ascii_lowercase();
    if let Ok(index) = NAMED_COLORS.binary_search_by(|(n, _)| (*n).cmp(lower.as_str())) {
        return NAMED_COLORS
            .get(index)
            .map(|(_, [r, g, b])| byte_color([*r, *g, *b, 255].map(u32::from)))
            .ok_or(ParseError::Invalid);
    }
    match lower.as_str() {
        "transparent" => Ok(Color::TRANSPARENT),
        "currentcolor" => Ok(Color::CurrentColor),
        _ => SystemColor::from_name(name)
            .map(Color::System)
            .ok_or(ParseError::Invalid),
    }
}

fn byte_color(rgba: [u32; 4]) -> Color {
    let [r, g, b, a] = rgba.map(|v| f64::from(v) / 255.0);
    legacy([r, g, b], a)
}

fn parse_hex(hex: &str) -> ParseResult<[u32; 4]> {
    let digits: Vec<u32> = hex
        .chars()
        .map(|c| c.to_digit(16))
        .collect::<Option<_>>()
        .ok_or(ParseError::Invalid)?;
    let (r, g, b, a) = match digits[..] {
        [r, g, b] => (r * 17, g * 17, b * 17, 255),
        [r, g, b, a] => (r * 17, g * 17, b * 17, a * 17),
        [r1, r2, g1, g2, b1, b2] => (r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2, 255),
        [r1, r2, g1, g2, b1, b2, a1, a2] => {
            (r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2, a1 * 16 + a2)
        }
        _ => return Err(ParseError::Invalid),
    };
    Ok([r, g, b, a])
}

#[derive(Clone, Copy, PartialEq)]
enum Channel {
    Number(f64),
    Percentage(f64),
    Angle(f64),
    None,
}

impl Channel {
    fn value(self, percent_reference: f64) -> Option<f64> {
        match self {
            Channel::Number(v) | Channel::Angle(v) => Some(v),
            Channel::Percentage(p) => Some(p / 100.0 * percent_reference),
            Channel::None => None,
        }
    }

    fn hue(self) -> ParseResult<Option<f64>> {
        match self {
            Channel::Number(v) | Channel::Angle(v) => Ok(Some(v)),
            Channel::None => Ok(None),
            Channel::Percentage(_) => Err(ParseError::Invalid),
        }
    }
}

fn parse_channel(input: &mut Parser<'_, '_>) -> ParseResult<Channel> {
    match input.next_token()? {
        Token::Number(n) => Ok(Channel::Number(n.value)),
        Token::Percentage(n) => Ok(Channel::Percentage(n.value)),
        Token::Dimension(n, unit) => angle_to_degrees(n.value as f32, &unit)
            .map(f64::from)
            .map(Channel::Angle)
            .ok_or(ParseError::Invalid),
        Token::Ident(ident) if ident.eq_ignore_ascii_case("none") => Ok(Channel::None),
        Token::Function(name) if is_math_function(&name) => {
            let (node, _) = CalcParser {
                percentage_as: None,
            }
            .parse_function_any(&name, input)?;
            match node.as_leaf() {
                Some(CalcLeaf::Number(v)) => Ok(Channel::Number(v.into())),
                Some(CalcLeaf::Percentage(v)) => Ok(Channel::Percentage(v.into())),
                Some(CalcLeaf::Angle(v)) => Ok(Channel::Angle(v.into())),
                _ => Err(ParseError::Invalid),
            }
        }
        _ => Err(ParseError::Invalid),
    }
}

fn parse_alpha(input: &mut Parser<'_, '_>) -> ParseResult<Option<f64>> {
    match parse_channel(input)? {
        Channel::Number(v) => Ok(Some(v.clamp(0.0, 1.0))),
        Channel::Percentage(p) => Ok(Some((p / 100.0).clamp(0.0, 1.0))),
        Channel::None => Ok(None),
        Channel::Angle(_) => Err(ParseError::Invalid),
    }
}

fn parse_modern(input: &mut Parser<'_, '_>) -> ParseResult<([Channel; 3], Option<f64>, bool)> {
    let channels = [
        parse_channel(input)?,
        parse_channel(input)?,
        parse_channel(input)?,
    ];
    let (alpha, alpha_missing) = if input.try_delim('/') {
        match parse_alpha(input)? {
            Some(a) => (Some(a), false),
            None => (None, true),
        }
    } else {
        (Some(1.0), false)
    };
    Ok((channels, alpha, alpha_missing))
}

fn parse_legacy(input: &mut Parser<'_, '_>, first: Channel) -> ParseResult<([Channel; 3], f64)> {
    input.expect_comma()?;
    let second = parse_channel(input)?;
    input.expect_comma()?;
    let third = parse_channel(input)?;
    let alpha = if input.try_parse(|p| p.expect_comma()).is_ok() {
        parse_alpha(input)?.ok_or(ParseError::Invalid)?
    } else {
        1.0
    };
    let channels = [first, second, third];
    if channels.contains(&Channel::None) {
        return Err(ParseError::Invalid);
    }
    Ok((channels, alpha))
}

fn is_comma_next(input: &mut Parser<'_, '_>) -> bool {
    let state = input.state();
    let comma = input.next_token() == Ok(Token::Comma);
    input.reset(&state);
    comma
}

fn absolute(space: ColorSpace, values: [Option<f64>; 3], alpha: Option<f64>) -> Color {
    let mut missing = 0;
    let mut components = [0.0; 3];
    for (i, value) in values.iter().enumerate() {
        match value {
            Some(v) => components[i] = *v,
            None => missing |= 1 << i,
        }
    }
    if alpha.is_none() {
        missing |= ALPHA_MISSING;
    }
    Color::Absolute(AbsoluteColor {
        space,
        components,
        alpha: alpha.unwrap_or(0.0),
        missing,
        legacy: false,
    })
}

fn legacy(rgb: [f64; 3], alpha: f64) -> Color {
    Color::Absolute(AbsoluteColor {
        space: ColorSpace::Srgb,
        components: rgb.map(|c| c.clamp(0.0, 1.0)),
        alpha,
        missing: 0,
        legacy: true,
    })
}

fn parse_function(name: &str, input: &mut Parser<'_, '_>) -> ParseResult<Color> {
    match name {
        "rgb" | "rgba" => {
            let first = parse_channel(input)?;
            let (channels, alpha) = if is_comma_next(input) {
                let (channels, alpha) = parse_legacy(input, first)?;
                let percent = matches!(first, Channel::Percentage(_));
                if channels
                    .iter()
                    .any(|c| matches!(c, Channel::Percentage(_)) != percent)
                {
                    return Err(ParseError::Invalid);
                }
                (channels, Some(alpha))
            } else {
                let (rest, alpha, _) = parse_rest_modern(input, first)?;
                (rest, alpha)
            };
            let mut rgb = [0.0; 3];
            for (out, channel) in rgb.iter_mut().zip(channels) {
                if let Channel::Angle(_) = channel {
                    return Err(ParseError::Invalid);
                }
                *out = channel.value(255.0).unwrap_or(0.0) / 255.0;
            }
            Ok(legacy(rgb, alpha.unwrap_or(0.0)))
        }
        "hsl" | "hsla" => {
            let first = parse_channel(input)?;
            let (channels, alpha) = if is_comma_next(input) {
                let (channels, alpha) = parse_legacy(input, first)?;
                if !matches!(channels[1], Channel::Percentage(_))
                    || !matches!(channels[2], Channel::Percentage(_))
                {
                    return Err(ParseError::Invalid);
                }
                (channels, Some(alpha))
            } else {
                let (rest, alpha, _) = parse_rest_modern(input, first)?;
                (rest, alpha)
            };
            let hue = channels[0].hue()?.unwrap_or(0.0);
            let s = number_or_percent(channels[1], 100.0)?.unwrap_or(0.0);
            let l = number_or_percent(channels[2], 100.0)?.unwrap_or(0.0);
            Ok(legacy(hsl_to_srgb([hue, s, l]), alpha.unwrap_or(0.0)))
        }
        "hwb" => {
            let (channels, alpha, _) = parse_modern(input)?;
            let hue = channels[0].hue()?.unwrap_or(0.0);
            let w = number_or_percent(channels[1], 100.0)?.unwrap_or(0.0);
            let b = number_or_percent(channels[2], 100.0)?.unwrap_or(0.0);
            Ok(legacy(hwb_to_srgb([hue, w, b]), alpha.unwrap_or(0.0)))
        }
        "lab" | "oklab" => {
            let (channels, alpha, _) = parse_modern(input)?;
            let (l_ref, ab_ref, space) = if name == "lab" {
                (100.0, 125.0, ColorSpace::Lab)
            } else {
                (1.0, 0.4, ColorSpace::Oklab)
            };
            Ok(absolute(
                space,
                [
                    number_or_percent(channels[0], l_ref)?,
                    number_or_percent(channels[1], ab_ref)?,
                    number_or_percent(channels[2], ab_ref)?,
                ],
                alpha,
            ))
        }
        "lch" | "oklch" => {
            let (channels, alpha, _) = parse_modern(input)?;
            let (l_ref, c_ref, space) = if name == "lch" {
                (100.0, 150.0, ColorSpace::Lch)
            } else {
                (1.0, 0.4, ColorSpace::Oklch)
            };
            let hue = channels[2].hue()?.map(|h| h.rem_euclid(360.0));
            Ok(absolute(
                space,
                [
                    number_or_percent(channels[0], l_ref)?,
                    number_or_percent(channels[1], c_ref)?,
                    hue,
                ],
                alpha,
            ))
        }
        "color" => parse_color_function(input),
        "device-cmyk" => parse_device_cmyk(input),
        "light-dark" => {
            let light = Color::parse(input)?;
            input.expect_comma()?;
            let dark = Color::parse(input)?;
            Ok(Color::LightDark(Box::new([light, dark])))
        }
        "color-mix" => parse_color_mix(input),
        _ => Err(ParseError::Invalid),
    }
}

fn parse_rest_modern(
    input: &mut Parser<'_, '_>,
    first: Channel,
) -> ParseResult<([Channel; 3], Option<f64>, bool)> {
    let channels = [first, parse_channel(input)?, parse_channel(input)?];
    let (alpha, missing) = if input.try_delim('/') {
        match parse_alpha(input)? {
            Some(a) => (Some(a), false),
            None => (Some(0.0), true),
        }
    } else {
        (Some(1.0), false)
    };
    Ok((channels, alpha, missing))
}

fn number_or_percent(channel: Channel, percent_reference: f64) -> ParseResult<Option<f64>> {
    match channel {
        Channel::Angle(_) => Err(ParseError::Invalid),
        other => Ok(other.value(percent_reference)),
    }
}

fn parse_color_function(input: &mut Parser<'_, '_>) -> ParseResult<Color> {
    let space_name = input.expect_ident()?;
    if space_name.starts_with("--") {
        let mut components = vec![];
        while let Ok(channel) = input.try_parse(parse_channel) {
            components.push(number_or_percent(channel, 1.0)?.ok_or(ParseError::Invalid)?);
        }
        let alpha = if input.try_delim('/') {
            parse_alpha(input)?.unwrap_or(0.0)
        } else {
            1.0
        };
        return Ok(Color::Profiled(Box::new(ProfiledColor {
            profile: space_name.into(),
            components: components.into_boxed_slice(),
            alpha,
        })));
    }
    let space = ColorSpace::from_name(&space_name)
        .filter(|s| s.is_predefined())
        .ok_or(ParseError::Invalid)?;
    let (channels, alpha, _) = parse_modern(input)?;
    Ok(absolute(
        space,
        [
            number_or_percent(channels[0], 1.0)?,
            number_or_percent(channels[1], 1.0)?,
            number_or_percent(channels[2], 1.0)?,
        ],
        alpha,
    ))
}

fn parse_device_cmyk(input: &mut Parser<'_, '_>) -> ParseResult<Color> {
    let first = parse_channel(input)?;
    let mut cmyk = [0.0; 5];
    if is_comma_next(input) {
        let mut channels = vec![first];
        for _ in 0..3 {
            input.expect_comma()?;
            channels.push(parse_channel(input)?);
        }
        for (out, channel) in cmyk.iter_mut().zip(&channels) {
            match channel {
                Channel::Number(v) => *out = v.clamp(0.0, 1.0),
                _ => return Err(ParseError::Invalid),
            }
        }
        cmyk[4] = if input.try_parse(|p| p.expect_comma()).is_ok() {
            parse_alpha(input)?.ok_or(ParseError::Invalid)?
        } else {
            1.0
        };
    } else {
        let channels = [
            first,
            parse_channel(input)?,
            parse_channel(input)?,
            parse_channel(input)?,
        ];
        for (out, channel) in cmyk.iter_mut().zip(channels) {
            *out = number_or_percent(channel, 1.0)?
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);
        }
        cmyk[4] = if input.try_delim('/') {
            parse_alpha(input)?.unwrap_or(0.0)
        } else {
            1.0
        };
    }
    Ok(Color::DeviceCmyk(Box::new(cmyk)))
}

fn parse_mix_item(input: &mut Parser<'_, '_>) -> ParseResult<(Color, Option<f64>)> {
    let percentage = |p: &mut Parser<'_, '_>| {
        p.try_parse(|p| crate::values::parse_percentage(p, crate::values::Range::All))
            .ok()
            .map(f64::from)
    };
    let mut amount = percentage(input);
    let color = Color::parse(input)?;
    if amount.is_none() {
        amount = percentage(input);
    }
    if amount.is_some_and(|a| !(0.0..=100.0).contains(&a)) {
        return Err(ParseError::Invalid);
    }
    Ok((color, amount))
}

fn parse_color_mix(input: &mut Parser<'_, '_>) -> ParseResult<Color> {
    input.expect_ident_matching("in")?;
    let space = ColorSpace::from_name(&input.expect_ident()?).ok_or(ParseError::Invalid)?;
    let hue = if space.hue_index().is_some() {
        let method = input.try_parse(|p| {
            let method = match p.expect_ident()?.to_ascii_lowercase().as_str() {
                "shorter" => HueMethod::Shorter,
                "longer" => HueMethod::Longer,
                "increasing" => HueMethod::Increasing,
                "decreasing" => HueMethod::Decreasing,
                _ => return Err(ParseError::Invalid),
            };
            p.expect_ident_matching("hue")?;
            Ok(method)
        });
        method.unwrap_or(HueMethod::Shorter)
    } else {
        HueMethod::Shorter
    };
    input.expect_comma()?;
    let (first, first_percentage) = parse_mix_item(input)?;
    input.expect_comma()?;
    let (second, second_percentage) = parse_mix_item(input)?;
    if first_percentage.unwrap_or(1.0) + second_percentage.unwrap_or(1.0) <= 0.0
        && first_percentage.is_some()
        && second_percentage.is_some()
    {
        return Err(ParseError::Invalid);
    }
    let mix = ColorMix {
        space,
        hue,
        first,
        first_percentage,
        second,
        second_percentage,
    };
    if let (Color::Absolute(a), Color::Absolute(b)) = (&mix.first, &mix.second)
        && let Some(mixed) = mix_absolute(&mix, a, b)
    {
        return Ok(Color::Absolute(mixed));
    }
    Ok(Color::Mix(Box::new(mix)))
}

fn mix_absolute(mix: &ColorMix, a: &AbsoluteColor, b: &AbsoluteColor) -> Option<AbsoluteColor> {
    let (p1, p2) = match (mix.first_percentage, mix.second_percentage) {
        (None, None) => (50.0, 50.0),
        (Some(p), None) => (p, 100.0 - p),
        (None, Some(p)) => (100.0 - p, p),
        (Some(p1), Some(p2)) => (p1, p2),
    };
    let sum = p1 + p2;
    if sum <= 0.0 {
        return None;
    }
    let (w2, multiplier) = (p2 / sum, (sum / 100.0).min(1.0));
    let mut ca = a.to_space(mix.space);
    let mut cb = b.to_space(mix.space);
    carry_missing(a, &mut ca);
    carry_missing(b, &mut cb);
    for i in 0..4 {
        let bit = 1 << i;
        let (ma, mb) = (ca.missing & bit != 0, cb.missing & bit != 0);
        if ma && !mb {
            set_component(&mut ca, i, component(&cb, i));
            ca.missing &= !bit;
        } else if mb && !ma {
            set_component(&mut cb, i, component(&ca, i));
            cb.missing &= !bit;
        }
    }
    if let Some(h) = mix.space.hue_index() {
        let (mut h1, mut h2) = (ca.components[h], cb.components[h]);
        let diff = h2 - h1;
        match mix.hue {
            HueMethod::Shorter if diff > 180.0 => h1 += 360.0,
            HueMethod::Shorter if diff < -180.0 => h2 += 360.0,
            HueMethod::Longer if (0.0..180.0).contains(&diff) => h1 += 360.0,
            HueMethod::Longer if (-180.0..=0.0).contains(&diff) && diff != 0.0 => h2 += 360.0,
            HueMethod::Increasing if h2 < h1 => h2 += 360.0,
            HueMethod::Decreasing if h1 < h2 => h1 += 360.0,
            _ => {}
        }
        ca.components[h] = h1;
        cb.components[h] = h2;
    }
    let alpha = ca.alpha * (1.0 - w2) + cb.alpha * w2;
    let mut components = [0.0; 3];
    for (i, out) in components.iter_mut().enumerate() {
        let premultiplied = mix.space.hue_index() != Some(i);
        let (x, y) = if premultiplied {
            (ca.components[i] * ca.alpha, cb.components[i] * cb.alpha)
        } else {
            (ca.components[i], cb.components[i])
        };
        let value = x * (1.0 - w2) + y * w2;
        *out = if premultiplied && alpha != 0.0 {
            value / alpha
        } else {
            value
        };
    }
    if let Some(h) = mix.space.hue_index() {
        components[h] = components[h].rem_euclid(360.0);
    }
    let both_missing = ca.missing & cb.missing;
    Some(AbsoluteColor {
        space: mix.space,
        components,
        alpha: alpha * multiplier,
        missing: both_missing,
        legacy: false,
    })
}

fn carry_missing(original: &AbsoluteColor, converted: &mut AbsoluteColor) {
    if original.space == converted.space {
        converted.missing = original.missing;
    }
}

fn component(color: &AbsoluteColor, i: usize) -> f64 {
    color.components.get(i).copied().unwrap_or(color.alpha)
}

fn set_component(color: &mut AbsoluteColor, i: usize, value: f64) {
    match color.components.get_mut(i) {
        Some(c) => *c = value,
        None => color.alpha = value,
    }
}

pub fn hsl_to_srgb([hue, saturation, lightness]: [f64; 3]) -> [f64; 3] {
    let s = saturation / 100.0;
    let l = lightness / 100.0;
    let h = hue.rem_euclid(360.0);
    let f = |n: f64| {
        let k = (n + h / 30.0) % 12.0;
        let a = s * l.min(1.0 - l);
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [f(0.0), f(8.0), f(4.0)]
}

pub fn hwb_to_srgb([hue, white, black]: [f64; 3]) -> [f64; 3] {
    let (w, b) = (white / 100.0, black / 100.0);
    if w + b >= 1.0 {
        let gray = w / (w + b);
        return [gray; 3];
    }
    hsl_to_srgb([hue, 100.0, 50.0]).map(|c| c * (1.0 - w - b) + w)
}

fn srgb_to_hsl([r, g, b]: [f64; 3]) -> [f64; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    let (mut h, mut s) = (0.0, 0.0);
    if d != 0.0 {
        s = if l == 0.0 || l == 1.0 {
            0.0
        } else {
            (max - l) / l.min(1.0 - l)
        };
        h = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        } * 60.0;
    }
    [h, s * 100.0, l * 100.0]
}

fn srgb_to_hwb(rgb: [f64; 3]) -> [f64; 3] {
    let [h, _, _] = srgb_to_hsl(rgb);
    let white = rgb[0].min(rgb[1]).min(rgb[2]);
    let black = 1.0 - rgb[0].max(rgb[1]).max(rgb[2]);
    [h, white * 100.0, black * 100.0]
}

type Matrix = [[f64; 3]; 3];

fn multiply(m: &Matrix, v: [f64; 3]) -> [f64; 3] {
    m.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
}

const SRGB_TO_XYZ: Matrix = [
    [0.41239079926595934, 0.357584339383878, 0.1804807884018343],
    [0.21263900587151027, 0.715168678767756, 0.07219231536073371],
    [0.01933081871559182, 0.11919477979462598, 0.9505321522496607],
];
const XYZ_TO_SRGB: Matrix = [
    [3.2409699419045226, -1.537383177570094, -0.4986107602930034],
    [-0.9692436362808796, 1.8759675015077202, 0.04155505740717559],
    [
        0.05563007969699366,
        -0.20397695888897652,
        1.0569715142428786,
    ],
];
const P3_TO_XYZ: Matrix = [
    [0.4865709486482162, 0.26566769316909306, 0.1982172852343625],
    [0.2289745640697488, 0.6917385218365064, 0.079286914093745],
    [0.0, 0.04511338185890264, 1.043944368900976],
];
const XYZ_TO_P3: Matrix = [
    [2.493496911941425, -0.9313836179191239, -0.40271078445071684],
    [
        -0.8294889695615747,
        1.7626640603183463,
        0.023624685841943577,
    ],
    [
        0.03584583024378447,
        -0.07617238926804182,
        0.9568845240076872,
    ],
];
const A98_TO_XYZ: Matrix = [
    [0.5766690429101305, 0.1855582379065463, 0.1882286462349947],
    [0.29734497525053605, 0.6273635662554661, 0.07529145849399788],
    [0.02703136138641234, 0.07068885253582723, 0.9913375368376388],
];
const XYZ_TO_A98: Matrix = [
    [
        2.0415879038107465,
        -0.5650069742788596,
        -0.34473135077832956,
    ],
    [-0.9692436362808795, 1.8759675015077202, 0.04155505740717557],
    [
        0.013444280632031142,
        -0.11836239223101838,
        1.0151749943912054,
    ],
];
const PROPHOTO_TO_XYZ_D50: Matrix = [
    [0.7977604896723027, 0.13518583717574031, 0.0313493495815248],
    [
        0.2880711282292934,
        0.7118432178101014,
        0.00008565396060525902,
    ],
    [0.0, 0.0, 0.8251046025104601],
];
const XYZ_D50_TO_PROPHOTO: Matrix = [
    [
        1.3457989731028281,
        -0.25558010007997534,
        -0.05110628506753401,
    ],
    [-0.5446224939028347, 1.5082327413132781, 0.02053603239147973],
    [0.0, 0.0, 1.2119675456389454],
];
const REC2020_TO_XYZ: Matrix = [
    [0.6369580483012914, 0.14461690358620832, 0.1688809751641721],
    [0.2627002120112671, 0.6779980715188708, 0.05930171646986196],
    [0.0, 0.028072693049087428, 1.060985057710791],
];
const XYZ_TO_REC2020: Matrix = [
    [1.716651187971268, -0.355670783776392, -0.25336628137366],
    [-0.666684351832489, 1.616481236634939, 0.0157685458139111],
    [0.017639857445311, -0.042770613257809, 0.942103121235474],
];
const D65_TO_D50: Matrix = [
    [
        1.0479298208405488,
        0.022946793341019088,
        -0.05019222954313557,
    ],
    [
        0.029627815688159344,
        0.990434484573249,
        -0.01707382502938514,
    ],
    [
        -0.009243058152591178,
        0.015055144896577895,
        0.7518742899580008,
    ],
];
const D50_TO_D65: Matrix = [
    [
        0.9554734527042182,
        -0.023098536874261423,
        0.0632593086610217,
    ],
    [
        -0.028369706963208136,
        1.0099954580058226,
        0.021041398966943008,
    ],
    [
        0.012314001688319899,
        -0.020507696433477912,
        1.3303659366080753,
    ],
];
const XYZ_TO_LMS: Matrix = [
    [0.819022437996703, 0.3619062600528904, -0.1288737815209879],
    [0.0329836539323885, 0.9292868615863434, 0.0361446663506424],
    [0.0481771893596242, 0.2642395317527308, 0.6335478284694309],
];
const LMS_TO_OKLAB: Matrix = [
    [0.210454268309314, 0.7936177747023054, -0.0040720430116193],
    [1.9779985324311684, -2.42859224204858, 0.450593709617411],
    [0.0259040424655478, 0.7827717124575296, -0.8086757549230774],
];
const OKLAB_TO_LMS: Matrix = [
    [1.0, 0.3963377773761749, 0.2158037573099136],
    [1.0, -0.1055613458156586, -0.0638541728258133],
    [1.0, -0.0894841775298119, -1.2914855480194092],
];
const LMS_TO_XYZ: Matrix = [
    [1.2268798758459243, -0.5578149944602171, 0.2813910456659647],
    [-0.0405757452148008, 1.112286803280317, -0.0717110580655164],
    [-0.0763729366746601, -0.4214933324022432, 1.5869240198367816],
];
const D50_WHITE: [f64; 3] = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];

fn signed(c: f64, f: impl Fn(f64) -> f64) -> f64 {
    c.signum() * f(c.abs())
}

fn srgb_to_linear(c: f64) -> f64 {
    signed(c, |c| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    })
}

fn linear_to_srgb(c: f64) -> f64 {
    signed(c, |c| {
        if c <= 0.0031308 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        }
    })
}

const REC_ALPHA: f64 = 1.09929682680944;
const REC_BETA: f64 = 0.018053968510807;

fn to_xyz_d65(space: ColorSpace, c: [f64; 3]) -> [f64; 3] {
    match space {
        ColorSpace::Srgb => multiply(&SRGB_TO_XYZ, c.map(srgb_to_linear)),
        ColorSpace::SrgbLinear => multiply(&SRGB_TO_XYZ, c),
        ColorSpace::DisplayP3 => multiply(&P3_TO_XYZ, c.map(srgb_to_linear)),
        ColorSpace::A98Rgb => {
            multiply(&A98_TO_XYZ, c.map(|v| signed(v, |v| v.powf(563.0 / 256.0))))
        }
        ColorSpace::ProphotoRgb => multiply(
            &D50_TO_D65,
            multiply(
                &PROPHOTO_TO_XYZ_D50,
                c.map(|v| {
                    signed(v, |v| {
                        if v <= 16.0 / 512.0 {
                            v / 16.0
                        } else {
                            v.powf(1.8)
                        }
                    })
                }),
            ),
        ),
        ColorSpace::Rec2020 => multiply(
            &REC2020_TO_XYZ,
            c.map(|v| {
                signed(v, |v| {
                    if v < REC_BETA * 4.5 {
                        v / 4.5
                    } else {
                        ((v + REC_ALPHA - 1.0) / REC_ALPHA).powf(1.0 / 0.45)
                    }
                })
            }),
        ),
        ColorSpace::XyzD65 => c,
        ColorSpace::XyzD50 => multiply(&D50_TO_D65, c),
        ColorSpace::Lab => multiply(&D50_TO_D65, lab_to_xyz_d50(c)),
        ColorSpace::Lch => multiply(&D50_TO_D65, lab_to_xyz_d50(lch_to_lab(c))),
        ColorSpace::Oklab => oklab_to_xyz(c),
        ColorSpace::Oklch => oklab_to_xyz(lch_to_lab(c)),
        ColorSpace::Hsl => to_xyz_d65(ColorSpace::Srgb, hsl_to_srgb(c)),
        ColorSpace::Hwb => to_xyz_d65(ColorSpace::Srgb, hwb_to_srgb(c)),
    }
}

fn from_xyz_d65(space: ColorSpace, xyz: [f64; 3]) -> [f64; 3] {
    match space {
        ColorSpace::Srgb => multiply(&XYZ_TO_SRGB, xyz).map(linear_to_srgb),
        ColorSpace::SrgbLinear => multiply(&XYZ_TO_SRGB, xyz),
        ColorSpace::DisplayP3 => multiply(&XYZ_TO_P3, xyz).map(linear_to_srgb),
        ColorSpace::A98Rgb => {
            multiply(&XYZ_TO_A98, xyz).map(|v| signed(v, |v| v.powf(256.0 / 563.0)))
        }
        ColorSpace::ProphotoRgb => {
            multiply(&XYZ_D50_TO_PROPHOTO, multiply(&D65_TO_D50, xyz)).map(|v| {
                signed(v, |v| {
                    if v < 1.0 / 512.0 {
                        v * 16.0
                    } else {
                        v.powf(1.0 / 1.8)
                    }
                })
            })
        }
        ColorSpace::Rec2020 => multiply(&XYZ_TO_REC2020, xyz).map(|v| {
            signed(v, |v| {
                if v < REC_BETA {
                    v * 4.5
                } else {
                    REC_ALPHA * v.powf(0.45) - (REC_ALPHA - 1.0)
                }
            })
        }),
        ColorSpace::XyzD65 => xyz,
        ColorSpace::XyzD50 => multiply(&D65_TO_D50, xyz),
        ColorSpace::Lab => xyz_d50_to_lab(multiply(&D65_TO_D50, xyz)),
        ColorSpace::Lch => lab_to_lch(xyz_d50_to_lab(multiply(&D65_TO_D50, xyz))),
        ColorSpace::Oklab => xyz_to_oklab(xyz),
        ColorSpace::Oklch => lab_to_lch(xyz_to_oklab(xyz)),
        ColorSpace::Hsl | ColorSpace::Hwb => {
            let rgb = from_xyz_d65(ColorSpace::Srgb, xyz);
            if space == ColorSpace::Hsl {
                srgb_to_hsl(rgb)
            } else {
                srgb_to_hwb(rgb)
            }
        }
    }
}

const KAPPA: f64 = 24389.0 / 27.0;
const EPSILON: f64 = 216.0 / 24389.0;

fn lab_to_xyz_d50([l, a, b]: [f64; 3]) -> [f64; 3] {
    let fy = (l + 16.0) / 116.0;
    let fx = a / 500.0 + fy;
    let fz = fy - b / 200.0;
    let x = if fx.powi(3) > EPSILON {
        fx.powi(3)
    } else {
        (116.0 * fx - 16.0) / KAPPA
    };
    let y = if l > KAPPA * EPSILON {
        fy.powi(3)
    } else {
        l / KAPPA
    };
    let z = if fz.powi(3) > EPSILON {
        fz.powi(3)
    } else {
        (116.0 * fz - 16.0) / KAPPA
    };
    [x * D50_WHITE[0], y * D50_WHITE[1], z * D50_WHITE[2]]
}

fn xyz_d50_to_lab(xyz: [f64; 3]) -> [f64; 3] {
    let f = |i: usize| {
        let v = xyz[i] / D50_WHITE[i];
        if v > EPSILON {
            v.cbrt()
        } else {
            (KAPPA * v + 16.0) / 116.0
        }
    };
    let (fx, fy, fz) = (f(0), f(1), f(2));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn lch_to_lab([l, c, h]: [f64; 3]) -> [f64; 3] {
    let h = h.to_radians();
    [l, c * h.cos(), c * h.sin()]
}

fn lab_to_lch([l, a, b]: [f64; 3]) -> [f64; 3] {
    let h = b.atan2(a).to_degrees().rem_euclid(360.0);
    [l, (a * a + b * b).sqrt(), h]
}

fn xyz_to_oklab(xyz: [f64; 3]) -> [f64; 3] {
    multiply(&LMS_TO_OKLAB, multiply(&XYZ_TO_LMS, xyz).map(f64::cbrt))
}

fn oklab_to_xyz(lab: [f64; 3]) -> [f64; 3] {
    multiply(&LMS_TO_XYZ, multiply(&OKLAB_TO_LMS, lab).map(|v| v.powi(3)))
}

pub fn write_number(out: &mut impl Write, value: impl Into<f64>) -> fmt::Result {
    let rounded = ((value.into() + 1e-7) * 1e6).round() / 1e6;
    if rounded == rounded.trunc() && rounded.abs() < 1e15 {
        write!(out, "{}", rounded as i64)
    } else {
        write!(out, "{rounded}")
    }
}

impl fmt::Display for AbsoluteColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let write_component = |f: &mut fmt::Formatter<'_>, i: usize| {
            if self.missing & (1 << i) != 0 {
                f.write_str("none")
            } else {
                write_number(f, self.components[i])
            }
        };
        if self.legacy {
            let has_alpha = self.alpha < 1.0;
            f.write_str(if has_alpha { "rgba(" } else { "rgb(" })?;
            for (i, c) in self.components.iter().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write_number(f, c * 255.0)?;
            }
            if has_alpha {
                f.write_str(", ")?;
                write_number(f, self.alpha)?;
            }
            return f.write_char(')');
        }
        if self.space.is_predefined() {
            write!(f, "color({} ", self.space.name())?;
        } else {
            write!(f, "{}(", self.space.name())?;
        }
        for i in 0..3 {
            if i > 0 {
                f.write_char(' ')?;
            }
            write_component(f, i)?;
        }
        if self.missing & ALPHA_MISSING != 0 {
            f.write_str(" / none")?;
        } else if self.alpha < 1.0 {
            f.write_str(" / ")?;
            write_number(f, self.alpha)?;
        }
        f.write_char(')')
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Color::CurrentColor => f.write_str("currentcolor"),
            Color::Absolute(color) => color.fmt(f),
            Color::System(system) => f.write_str(system.name()),
            Color::LightDark(pair) => write!(f, "light-dark({}, {})", pair[0], pair[1]),
            Color::Mix(mix) => {
                write!(f, "color-mix(in {}", mix.space.name())?;
                if mix.hue != HueMethod::Shorter {
                    let method = match mix.hue {
                        HueMethod::Longer => "longer",
                        HueMethod::Increasing => "increasing",
                        HueMethod::Decreasing => "decreasing",
                        HueMethod::Shorter => "shorter",
                    };
                    write!(f, " {method} hue")?;
                }
                for (color, percentage) in [
                    (&mix.first, mix.first_percentage),
                    (&mix.second, mix.second_percentage),
                ] {
                    write!(f, ", {color}")?;
                    if let Some(p) = percentage {
                        f.write_char(' ')?;
                        write_number(f, p)?;
                        f.write_char('%')?;
                    }
                }
                f.write_char(')')
            }
            Color::DeviceCmyk(cmyk) => {
                f.write_str("color(device-cmyk")?;
                for c in &cmyk[..4] {
                    f.write_char(' ')?;
                    write_number(f, *c)?;
                }
                if cmyk[4] < 1.0 {
                    f.write_str(" / ")?;
                    write_number(f, cmyk[4])?;
                }
                f.write_char(')')
            }
            Color::Profiled(color) => {
                write!(f, "color({}", color.profile)?;
                for c in &color.components {
                    f.write_char(' ')?;
                    write_number(f, *c)?;
                }
                if color.alpha < 1.0 {
                    f.write_str(" / ")?;
                    write_number(f, color.alpha)?;
                }
                f.write_char(')')
            }
        }
    }
}
