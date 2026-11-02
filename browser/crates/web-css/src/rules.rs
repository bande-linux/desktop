use std::borrow::Cow;

use crate::parser::{Delimiters, ParseError, ParseResult, Parser};
use crate::tokenizer::Token;

pub trait RuleHandler<'i> {
    type AtPrelude;
    type QualifiedPrelude;
    type Item;

    fn at_rule_prelude(
        &mut self,
        _name: Cow<'i, str>,
        _input: &mut Parser<'i, '_>,
    ) -> ParseResult<Self::AtPrelude> {
        Err(ParseError::Invalid)
    }

    fn at_rule_block(
        &mut self,
        _prelude: Self::AtPrelude,
        _input: &mut Parser<'i, '_>,
    ) -> ParseResult<Self::Item> {
        Err(ParseError::Invalid)
    }

    fn at_rule_without_block(&mut self, _prelude: Self::AtPrelude) -> ParseResult<Self::Item> {
        Err(ParseError::Invalid)
    }

    fn qualified_rule_prelude(
        &mut self,
        _input: &mut Parser<'i, '_>,
    ) -> ParseResult<Self::QualifiedPrelude> {
        Err(ParseError::Invalid)
    }

    fn qualified_rule_block(
        &mut self,
        _prelude: Self::QualifiedPrelude,
        _input: &mut Parser<'i, '_>,
    ) -> ParseResult<Self::Item> {
        Err(ParseError::Invalid)
    }

    fn declaration(
        &mut self,
        _name: Cow<'i, str>,
        _input: &mut Parser<'i, '_>,
    ) -> ParseResult<Self::Item> {
        Err(ParseError::Invalid)
    }
}

pub fn parse_important(input: &mut Parser<'_, '_>) -> ParseResult<()> {
    input.expect_delim('!')?;
    input.expect_ident_matching("important")
}

fn parse_at_rule<'i, H: RuleHandler<'i>>(
    input: &mut Parser<'i, '_>,
    handler: &mut H,
    name: Cow<'i, str>,
) -> ParseResult<H::Item> {
    let prelude = input.parse_until_before(Delimiters::SEMICOLON | Delimiters::CURLY_OPEN, |p| {
        handler.at_rule_prelude(name, p)
    });
    match input.next_including_whitespace() {
        Ok(Token::OpenCurly) => {
            let prelude = prelude?;
            input.parse_nested_block(|p| handler.at_rule_block(prelude, p))
        }
        _ => handler.at_rule_without_block(prelude?),
    }
}

fn parse_qualified_rule<'i, H: RuleHandler<'i>>(
    input: &mut Parser<'i, '_>,
    handler: &mut H,
    nested: bool,
) -> ParseResult<H::Item> {
    let delimiters = if nested {
        Delimiters::SEMICOLON | Delimiters::CURLY_OPEN
    } else {
        Delimiters::CURLY_OPEN
    };
    let prelude = input.parse_until_before(delimiters, |p| handler.qualified_rule_prelude(p));
    match input.next_including_whitespace() {
        Ok(Token::OpenCurly) => {
            let prelude = prelude?;
            input.parse_nested_block(|p| handler.qualified_rule_block(prelude, p))
        }
        _ => Err(ParseError::Invalid),
    }
}

pub struct RuleListParser<'i, 't, 'a, H> {
    input: &'a mut Parser<'i, 't>,
    handler: &'a mut H,
    top_level: bool,
}

impl<'i, 't, 'a, H: RuleHandler<'i>> RuleListParser<'i, 't, 'a, H> {
    pub fn new_for_stylesheet(input: &'a mut Parser<'i, 't>, handler: &'a mut H) -> Self {
        RuleListParser {
            input,
            handler,
            top_level: true,
        }
    }

    pub fn new_for_nested_rules(input: &'a mut Parser<'i, 't>, handler: &'a mut H) -> Self {
        RuleListParser {
            input,
            handler,
            top_level: false,
        }
    }
}

impl<'i, H: RuleHandler<'i>> Iterator for RuleListParser<'i, '_, '_, H> {
    type Item = ParseResult<H::Item>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.input.skip_whitespace();
            let start = self.input.state();
            match self.input.next_including_whitespace().ok()? {
                Token::Cdo | Token::Cdc if self.top_level => continue,
                Token::AtKeyword(name) => {
                    return Some(parse_at_rule(self.input, self.handler, name));
                }
                _ => {
                    self.input.reset(&start);
                    return Some(parse_qualified_rule(self.input, self.handler, false));
                }
            }
        }
    }
}

pub struct BlockContentsParser<'i, 't, 'a, H> {
    input: &'a mut Parser<'i, 't>,
    handler: &'a mut H,
}

impl<'i, 't, 'a, H: RuleHandler<'i>> BlockContentsParser<'i, 't, 'a, H> {
    pub fn new(input: &'a mut Parser<'i, 't>, handler: &'a mut H) -> Self {
        BlockContentsParser { input, handler }
    }
}

impl<'i, H: RuleHandler<'i>> Iterator for BlockContentsParser<'i, '_, '_, H> {
    type Item = ParseResult<H::Item>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.input.skip_whitespace();
            let start = self.input.state();
            match self.input.next_including_whitespace().ok()? {
                Token::Semicolon => continue,
                Token::AtKeyword(name) => {
                    return Some(parse_at_rule(self.input, self.handler, name));
                }
                Token::Ident(name) => {
                    let handler = &mut *self.handler;
                    let declaration = self.input.parse_until_after(Delimiters::SEMICOLON, |p| {
                        p.expect_token(Token::Colon)?;
                        handler.declaration(name, p)
                    });
                    if declaration.is_ok() {
                        return Some(declaration);
                    }
                    self.input.reset(&start);
                    return Some(parse_qualified_rule(self.input, self.handler, true));
                }
                _ => {
                    self.input.reset(&start);
                    return Some(parse_qualified_rule(self.input, self.handler, true));
                }
            }
        }
    }
}
