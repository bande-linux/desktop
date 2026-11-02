use crate::parser::{Delimiters, ParseError, ParseResult, Parser};
use crate::tokenizer::{Token, Tokenizer};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct FontFace {
    pub family: Box<str>,
    pub sources: Vec<FontSource>,
    pub weight: Option<(u16, u16)>,
    pub italic: Option<bool>,
    pub ranges: Box<[(u32, u32)]>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FontSource {
    Url(Box<str>),
    Local(Box<str>),
}

impl FontFace {
    pub fn covers(&self, ch: char) -> bool {
        let code = ch as u32;
        self.ranges.is_empty()
            || self
                .ranges
                .iter()
                .any(|&(start, end)| (start..=end).contains(&code))
    }
}

pub(crate) fn descriptor(
    face: &mut FontFace,
    name: &str,
    input: &mut Parser<'_, '_>,
) -> ParseResult<()> {
    match name {
        "font-family" => face.family = family(input)?,
        "src" => face.sources = sources(input)?,
        "font-weight" => face.weight = Some(weight(input)?),
        "font-style" => face.italic = Some(style(input)?),
        "unicode-range" => {
            input.skip_whitespace();
            let start = input.position();
            while input.next_including_whitespace().is_ok() {}
            let text = input.slice_from(start);
            let mut tokenizer = Tokenizer::with_unicode_ranges(text);
            let mut ranges = Parser::new(&mut tokenizer);
            face.ranges = ranges
                .parse_comma_separated(|input| match input.next_token()? {
                    Token::UnicodeRange(start, end) if start <= end => Ok((start, end)),
                    _ => Err(ParseError::Invalid),
                })?
                .into_boxed_slice()
        }
        _ => while input.next_including_whitespace().is_ok() {},
    }
    input.expect_exhausted()
}

fn family(input: &mut Parser<'_, '_>) -> ParseResult<Box<str>> {
    if let Ok(name) = input.try_parse(|input| input.expect_string()) {
        return Ok(name.into());
    }
    let mut words = Vec::new();
    while let Ok(word) = input.try_parse(|input| input.expect_ident()) {
        words.push(word);
    }
    if words.is_empty() {
        return Err(ParseError::Invalid);
    }
    Ok(words.join(" ").into())
}

fn sources(input: &mut Parser<'_, '_>) -> ParseResult<Vec<FontSource>> {
    let mut sources = Vec::new();
    loop {
        let parsed = input.parse_until_before(Delimiters::COMMA, |input| {
            let source = source(input);
            while input.next_including_whitespace().is_ok() {}
            source
        });
        if let Ok(Some(source)) = parsed {
            sources.push(source);
        }
        match input.next_token() {
            Ok(Token::Comma) => {}
            _ => break,
        }
    }
    if sources.is_empty() {
        return Err(ParseError::Invalid);
    }
    Ok(sources)
}

fn source(input: &mut Parser<'_, '_>) -> ParseResult<Option<FontSource>> {
    if input
        .try_parse(|input| input.expect_function_matching("local"))
        .is_ok()
    {
        let name = input.parse_nested_block(|input| family(input))?;
        return Ok(Some(FontSource::Local(name)));
    }
    let url = match input.next_token()? {
        Token::Url(url) => url,
        Token::Function(name) if name.eq_ignore_ascii_case("url") => {
            input.parse_nested_block(|input| input.expect_string())?
        }
        _ => return Err(ParseError::Invalid),
    };
    let mut supported = true;
    while let Ok(function) = input.try_parse(|input| input.expect_function()) {
        let lowered = function.to_ascii_lowercase();
        let ok = input.parse_nested_block(|input| {
            let mut ok = false;
            while let Ok(token) = input.next_token() {
                ok |= match (lowered.as_str(), &token) {
                    ("format", Token::String(value) | Token::Ident(value)) => matches!(
                        value.to_ascii_lowercase().as_str(),
                        "woff2" | "woff" | "truetype" | "opentype"
                    ),
                    ("tech", Token::Ident(value)) => matches!(
                        value.to_ascii_lowercase().as_str(),
                        "variations" | "features-opentype" | "color-colrv0" | "color-colrv1"
                    ),
                    _ => false,
                };
            }
            Ok(ok)
        })?;
        supported &= ok;
    }
    Ok(supported.then(|| FontSource::Url(url.into())))
}

fn weight_value(input: &mut Parser<'_, '_>) -> ParseResult<u16> {
    if input.try_ident_matching("normal") {
        return Ok(400);
    }
    if input.try_ident_matching("bold") {
        return Ok(700);
    }
    let number = input.expect_number()?;
    if !(1.0..=1000.0).contains(&number.value) {
        return Err(ParseError::Invalid);
    }
    Ok(number.value.round() as u16)
}

fn weight(input: &mut Parser<'_, '_>) -> ParseResult<(u16, u16)> {
    if input.try_ident_matching("auto") {
        return Ok((1, 1000));
    }
    let start = weight_value(input)?;
    let end = input.try_parse(weight_value).unwrap_or(start);
    Ok((start.min(end), start.max(end)))
}

fn style(input: &mut Parser<'_, '_>) -> ParseResult<bool> {
    let italic = match input.expect_ident()?.to_ascii_lowercase().as_str() {
        "normal" => false,
        "italic" | "oblique" => true,
        "auto" => false,
        _ => return Err(ParseError::Invalid),
    };
    while input.next_including_whitespace().is_ok() {}
    Ok(italic)
}
