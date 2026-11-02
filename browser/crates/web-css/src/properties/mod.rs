mod shorthands;

use std::fmt;
use std::sync::Arc;

use web_atoms::{Atom, AtomTable};

use crate::parser::{Delimiters, ParseError, ParseResult, Parser};
use crate::rules::parse_important;
use crate::tokenizer::{Token, Tokenizer, preprocess};
use crate::values::LengthPercentage;
use crate::values::Parse;
use crate::values::color::Color;
use crate::values::specified::*;

pub type Expanded = Vec<(LonghandId, Option<PropertyValue>)>;

include!(concat!(env!("OUT_DIR"), "/properties.rs"));

impl PropertyId {
    pub fn from_name(name: &str) -> Option<PropertyId> {
        if name.bytes().any(|b| b.is_ascii_uppercase()) {
            Self::from_lowercase(&name.to_ascii_lowercase())
        } else {
            Self::from_lowercase(name)
        }
    }

    fn longhands(&self) -> &'static [LonghandId] {
        match self {
            PropertyId::Longhand(id) => std::slice::from_ref(&LonghandId::ALL[*id as usize]),
            PropertyId::Shorthand(id) => id.longhands(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CssWideKeyword {
    Initial,
    Inherit,
    Unset,
    Revert,
    RevertLayer,
}

impl CssWideKeyword {
    fn from_ident(ident: &str) -> Option<CssWideKeyword> {
        [
            ("initial", CssWideKeyword::Initial),
            ("inherit", CssWideKeyword::Inherit),
            ("unset", CssWideKeyword::Unset),
            ("revert", CssWideKeyword::Revert),
            ("revert-layer", CssWideKeyword::RevertLayer),
        ]
        .into_iter()
        .find(|(name, _)| ident.eq_ignore_ascii_case(name))
        .map(|(_, keyword)| keyword)
    }

    pub fn name(self) -> &'static str {
        match self {
            CssWideKeyword::Initial => "initial",
            CssWideKeyword::Inherit => "inherit",
            CssWideKeyword::Unset => "unset",
            CssWideKeyword::Revert => "revert",
            CssWideKeyword::RevertLayer => "revert-layer",
        }
    }

    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<CssWideKeyword> {
        input.try_parse(|p| {
            let keyword =
                CssWideKeyword::from_ident(&p.expect_ident()?).ok_or(ParseError::Invalid)?;
            p.expect_exhausted()?;
            Ok(keyword)
        })
    }
}

#[derive(Debug, PartialEq)]
pub struct UnparsedValue {
    pub css: Box<str>,
    pub shorthand: Option<ShorthandId>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DeclaredValue {
    Value(PropertyValue),
    CssWide(CssWideKeyword),
    Unparsed(Arc<UnparsedValue>),
}

impl fmt::Display for DeclaredValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeclaredValue::Value(value) => value.fmt(f),
            DeclaredValue::CssWide(keyword) => f.write_str(keyword.name()),
            DeclaredValue::Unparsed(unparsed) => f.write_str(&unparsed.css),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub id: LonghandId,
    pub value: DeclaredValue,
    pub important: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CustomValue {
    Value(Arc<str>),
    CssWide(CssWideKeyword),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CustomDeclaration {
    pub name: Atom,
    pub value: CustomValue,
    pub important: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeclarationBlock {
    pub declarations: Vec<Declaration>,
    pub custom: Vec<CustomDeclaration>,
}

fn contains_var(input: &mut Parser<'_, '_>) -> bool {
    while let Ok(token) = input.next_including_whitespace() {
        let found = match &token {
            Token::Function(name)
                if name.eq_ignore_ascii_case("var") || name.eq_ignore_ascii_case("env") =>
            {
                true
            }
            token if token.block_end().is_some() => {
                input.parse_nested_block(|p| Ok(contains_var(p))) == Ok(true)
            }
            _ => false,
        };
        if found {
            return true;
        }
    }
    false
}

fn custom_value_is_valid(input: &mut Parser<'_, '_>) -> bool {
    let mut curly = false;
    let mut other = false;
    while let Ok(token) = input.next_including_whitespace() {
        match token {
            Token::BadString
            | Token::BadUrl
            | Token::CloseParen
            | Token::CloseSquare
            | Token::CloseCurly => {
                return false;
            }
            Token::OpenCurly => curly = true,
            Token::Whitespace => {}
            _ => other = true,
        }
        if token.block_end().is_some()
            && input.parse_nested_block(|p| Ok(nested_is_valid(p))) != Ok(true)
        {
            return false;
        }
    }
    !(curly && other)
}

fn nested_is_valid(input: &mut Parser<'_, '_>) -> bool {
    while let Ok(token) = input.next_including_whitespace() {
        match token {
            Token::BadString
            | Token::BadUrl
            | Token::CloseParen
            | Token::CloseSquare
            | Token::CloseCurly => {
                return false;
            }
            token
                if token.block_end().is_some()
                    && input.parse_nested_block(|p| Ok(nested_is_valid(p))) != Ok(true) =>
            {
                return false;
            }
            _ => {}
        }
    }
    true
}

pub fn parse_property_value(
    id: PropertyId,
    input: &mut Parser<'_, '_>,
) -> ParseResult<Vec<(LonghandId, DeclaredValue)>> {
    if let Ok(keyword) = CssWideKeyword::parse(input) {
        return Ok(id
            .longhands()
            .iter()
            .map(|longhand| (*longhand, DeclaredValue::CssWide(keyword)))
            .collect());
    }
    let start = input.state();
    let typed = match id {
        PropertyId::Longhand(longhand) => parse_longhand(longhand, input).and_then(|value| {
            input.expect_exhausted()?;
            Ok(vec![(longhand, DeclaredValue::Value(value))])
        }),
        PropertyId::Shorthand(shorthand) => {
            let mut expanded = Expanded::new();
            parse_shorthand(shorthand, input, &mut expanded).and_then(|()| {
                input.expect_exhausted()?;
                Ok(expanded
                    .into_iter()
                    .map(|(longhand, value)| {
                        let value = value.map_or(
                            DeclaredValue::CssWide(CssWideKeyword::Initial),
                            DeclaredValue::Value,
                        );
                        (longhand, value)
                    })
                    .collect())
            })
        }
    };
    if typed.is_ok() {
        return typed;
    }
    input.reset(&start);
    input.skip_whitespace();
    let value_start = input.position();
    if !contains_var(input) {
        return Err(ParseError::Invalid);
    }
    while input.next_including_whitespace().is_ok() {}
    let css = input.slice_from(value_start).trim_end();
    let unparsed = Arc::new(UnparsedValue {
        css: css.into(),
        shorthand: match id {
            PropertyId::Shorthand(shorthand) => Some(shorthand),
            PropertyId::Longhand(_) => None,
        },
    });
    Ok(id
        .longhands()
        .iter()
        .map(|longhand| (*longhand, DeclaredValue::Unparsed(unparsed.clone())))
        .collect())
}

pub fn parse_value_str(id: PropertyId, css: &str) -> ParseResult<Vec<(LonghandId, DeclaredValue)>> {
    let css = preprocess(css);
    let mut tokenizer = Tokenizer::new(&css);
    let mut input = Parser::new(&mut tokenizer);
    parse_property_value(id, &mut input)
}

impl DeclarationBlock {
    pub fn is_empty(&self) -> bool {
        self.declarations.is_empty() && self.custom.is_empty()
    }

    pub fn parse_declaration(
        &mut self,
        name: &str,
        input: &mut Parser<'_, '_>,
        atoms: &mut AtomTable,
    ) -> ParseResult<()> {
        if name.starts_with("--") {
            let value = input.parse_until_before(Delimiters::BANG, |p| {
                if let Ok(keyword) = CssWideKeyword::parse(p) {
                    return Ok(CustomValue::CssWide(keyword));
                }
                p.skip_whitespace();
                let start = p.position();
                if !custom_value_is_valid(p) {
                    return Err(ParseError::Invalid);
                }
                Ok(CustomValue::Value(p.slice_from(start).trim_end().into()))
            })?;
            let important = input.try_parse(parse_important).is_ok();
            input.expect_exhausted()?;
            let name = atoms.intern(name);
            self.custom
                .retain(|c| c.name != name || (c.important && !important));
            self.custom.push(CustomDeclaration {
                name,
                value,
                important,
            });
            return Ok(());
        }
        let id = PropertyId::from_name(name).ok_or(ParseError::Invalid)?;
        let values = input.parse_until_before(Delimiters::BANG, |p| parse_property_value(id, p))?;
        let important = input.try_parse(parse_important).is_ok();
        input.expect_exhausted()?;
        for (id, value) in values {
            self.push(Declaration {
                id,
                value,
                important,
            });
        }
        Ok(())
    }

    fn push(&mut self, declaration: Declaration) {
        if let Some(index) = self
            .declarations
            .iter()
            .position(|d| d.id == declaration.id)
        {
            if self.declarations[index].important && !declaration.important {
                return;
            }
            self.declarations.remove(index);
        }
        self.declarations.push(declaration);
    }
}
