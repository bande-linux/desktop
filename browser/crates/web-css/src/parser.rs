use std::borrow::Cow;

use crate::tokenizer::{Numeric, Token, Tokenizer};

const MAX_NESTING: u16 = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    EndOfInput,
    Unexpected,
    Invalid,
    TooDeep,
}

pub type ParseResult<T> = Result<T, ParseError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Paren,
    Square,
    Curly,
}

impl BlockKind {
    fn of(token: &Token<'_>) -> Option<BlockKind> {
        match token {
            Token::Function(_) | Token::OpenParen => Some(BlockKind::Paren),
            Token::OpenSquare => Some(BlockKind::Square),
            Token::OpenCurly => Some(BlockKind::Curly),
            _ => None,
        }
    }

    fn closed_by(self, token: &Token<'_>) -> bool {
        matches!(
            (self, token),
            (BlockKind::Paren, Token::CloseParen)
                | (BlockKind::Square, Token::CloseSquare)
                | (BlockKind::Curly, Token::CloseCurly)
        )
    }

    fn closer(self) -> Delimiters {
        match self {
            BlockKind::Paren => Delimiters::CLOSE_PAREN,
            BlockKind::Square => Delimiters::CLOSE_SQUARE,
            BlockKind::Curly => Delimiters::CLOSE_CURLY,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delimiters(u8);

impl Delimiters {
    pub const NONE: Delimiters = Delimiters(0);
    pub const SEMICOLON: Delimiters = Delimiters(1);
    pub const BANG: Delimiters = Delimiters(1 << 1);
    pub const COMMA: Delimiters = Delimiters(1 << 2);
    pub const CURLY_OPEN: Delimiters = Delimiters(1 << 3);
    const CLOSE_CURLY: Delimiters = Delimiters(1 << 4);
    const CLOSE_SQUARE: Delimiters = Delimiters(1 << 5);
    const CLOSE_PAREN: Delimiters = Delimiters(1 << 6);

    fn of_byte(b: Option<u8>) -> Delimiters {
        match b {
            Some(b';') => Delimiters::SEMICOLON,
            Some(b'!') => Delimiters::BANG,
            Some(b',') => Delimiters::COMMA,
            Some(b'{') => Delimiters::CURLY_OPEN,
            Some(b'}') => Delimiters::CLOSE_CURLY,
            Some(b']') => Delimiters::CLOSE_SQUARE,
            Some(b')') => Delimiters::CLOSE_PAREN,
            _ => Delimiters::NONE,
        }
    }

    fn contains(self, other: Delimiters) -> bool {
        self.0 & other.0 != 0
    }
}

impl std::ops::BitOr for Delimiters {
    type Output = Delimiters;

    fn bitor(self, other: Delimiters) -> Delimiters {
        Delimiters(self.0 | other.0)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ParserState {
    position: usize,
    pending_block: Option<BlockKind>,
}

impl ParserState {
    pub fn position(&self) -> usize {
        self.position
    }
}

pub struct Parser<'i, 't> {
    tokenizer: &'t mut Tokenizer<'i>,
    pending_block: Option<BlockKind>,
    stop: Delimiters,
    depth: u16,
}

fn skip_block(tokenizer: &mut Tokenizer<'_>, kind: BlockKind) {
    let mut open = vec![];
    let mut current = kind;
    while let Some(token) = tokenizer.next_token() {
        if let Some(inner) = BlockKind::of(&token) {
            open.push(current);
            current = inner;
        } else if current.closed_by(&token) {
            match open.pop() {
                Some(outer) => current = outer,
                None => return,
            }
        }
    }
}

impl<'i, 't> Parser<'i, 't> {
    pub fn new(tokenizer: &'t mut Tokenizer<'i>) -> Self {
        Parser {
            tokenizer,
            pending_block: None,
            stop: Delimiters::NONE,
            depth: 0,
        }
    }

    pub fn position(&self) -> usize {
        self.tokenizer.position()
    }

    pub fn slice_from(&self, start: usize) -> &'i str {
        self.tokenizer.slice(start, self.tokenizer.position())
    }

    pub fn state(&self) -> ParserState {
        ParserState {
            position: self.tokenizer.position(),
            pending_block: self.pending_block,
        }
    }

    pub fn reset(&mut self, state: &ParserState) {
        self.tokenizer.reset(state.position);
        self.pending_block = state.pending_block;
    }

    pub fn try_parse<T>(&mut self, f: impl FnOnce(&mut Self) -> ParseResult<T>) -> ParseResult<T> {
        let state = self.state();
        let result = f(self);
        if result.is_err() {
            self.reset(&state);
        }
        result
    }

    fn skip_pending_block(&mut self) {
        if let Some(kind) = self.pending_block.take() {
            skip_block(self.tokenizer, kind);
        }
    }

    pub fn next_including_whitespace(&mut self) -> ParseResult<Token<'i>> {
        self.skip_pending_block();
        self.tokenizer.skip_comments();
        if self
            .stop
            .contains(Delimiters::of_byte(self.tokenizer.next_byte()))
        {
            return Err(ParseError::EndOfInput);
        }
        let token = self.tokenizer.next_token().ok_or(ParseError::EndOfInput)?;
        self.pending_block = BlockKind::of(&token);
        Ok(token)
    }

    pub fn skip_whitespace(&mut self) {
        self.skip_pending_block();
        self.tokenizer.skip_whitespace();
    }

    pub fn next_token(&mut self) -> ParseResult<Token<'i>> {
        self.skip_whitespace();
        self.next_including_whitespace()
    }

    pub fn is_exhausted(&mut self) -> bool {
        let state = self.state();
        let exhausted = self.next_token().is_err();
        self.reset(&state);
        exhausted
    }

    pub fn expect_exhausted(&mut self) -> ParseResult<()> {
        if self.is_exhausted() {
            Ok(())
        } else {
            Err(ParseError::Unexpected)
        }
    }

    pub fn at_delimiter(&mut self, delimiters: Delimiters) -> bool {
        self.skip_whitespace();
        delimiters.contains(Delimiters::of_byte(self.tokenizer.next_byte()))
    }

    pub fn parse_nested_block<T>(
        &mut self,
        f: impl FnOnce(&mut Parser<'i, '_>) -> ParseResult<T>,
    ) -> ParseResult<T> {
        let kind = self.pending_block.take().ok_or(ParseError::Unexpected)?;
        if self.depth >= MAX_NESTING {
            skip_block(self.tokenizer, kind);
            return Err(ParseError::TooDeep);
        }
        let mut nested = Parser {
            tokenizer: &mut *self.tokenizer,
            pending_block: None,
            stop: kind.closer(),
            depth: self.depth + 1,
        };
        let result = f(&mut nested);
        nested.skip_pending_block();
        skip_block(self.tokenizer, kind);
        result
    }

    pub fn parse_until_before<T>(
        &mut self,
        delimiters: Delimiters,
        f: impl FnOnce(&mut Parser<'i, '_>) -> ParseResult<T>,
    ) -> ParseResult<T> {
        self.skip_pending_block();
        let mut delimited = Parser {
            tokenizer: &mut *self.tokenizer,
            pending_block: None,
            stop: self.stop | delimiters,
            depth: self.depth,
        };
        let result = f(&mut delimited);
        while delimited.next_including_whitespace().is_ok() {}
        result
    }

    pub fn parse_until_after<T>(
        &mut self,
        delimiters: Delimiters,
        f: impl FnOnce(&mut Parser<'i, '_>) -> ParseResult<T>,
    ) -> ParseResult<T> {
        let result = self.parse_until_before(delimiters, f);
        let next = Delimiters::of_byte(self.tokenizer.next_byte());
        if delimiters.contains(next) && !self.stop.contains(next) {
            let _ = self.tokenizer.next_token();
        }
        result
    }

    pub fn parse_comma_separated<T>(
        &mut self,
        mut f: impl FnMut(&mut Parser<'i, '_>) -> ParseResult<T>,
    ) -> ParseResult<Vec<T>> {
        let mut values = vec![];
        loop {
            values.push(self.parse_until_before(Delimiters::COMMA, |p| {
                let value = f(p)?;
                p.expect_exhausted()?;
                Ok(value)
            })?);
            match self.next_token() {
                Err(_) => return Ok(values),
                Ok(Token::Comma) => {}
                Ok(_) => return Err(ParseError::Unexpected),
            }
        }
    }

    pub fn expect_ident(&mut self) -> ParseResult<Cow<'i, str>> {
        match self.next_token()? {
            Token::Ident(name) => Ok(name),
            _ => Err(ParseError::Unexpected),
        }
    }

    pub fn expect_ident_matching(&mut self, name: &str) -> ParseResult<()> {
        match self.next_token()? {
            Token::Ident(ident) if ident.eq_ignore_ascii_case(name) => Ok(()),
            _ => Err(ParseError::Unexpected),
        }
    }

    pub fn try_ident_matching(&mut self, name: &str) -> bool {
        self.try_parse(|p| p.expect_ident_matching(name)).is_ok()
    }

    pub fn expect_string(&mut self) -> ParseResult<Cow<'i, str>> {
        match self.next_token()? {
            Token::String(value) => Ok(value),
            _ => Err(ParseError::Unexpected),
        }
    }

    pub fn expect_url_or_string(&mut self) -> ParseResult<Cow<'i, str>> {
        match self.next_token()? {
            Token::Url(value) | Token::String(value) => Ok(value),
            Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                self.parse_nested_block(|p| p.expect_string())
            }
            _ => Err(ParseError::Unexpected),
        }
    }

    pub fn expect_number(&mut self) -> ParseResult<Numeric> {
        match self.next_token()? {
            Token::Number(number) => Ok(number),
            _ => Err(ParseError::Unexpected),
        }
    }

    pub fn expect_integer(&mut self) -> ParseResult<i32> {
        self.expect_number()?
            .int_value()
            .ok_or(ParseError::Unexpected)
    }

    pub fn expect_token(&mut self, expected: Token<'_>) -> ParseResult<()> {
        if self.next_token()? == expected {
            Ok(())
        } else {
            Err(ParseError::Unexpected)
        }
    }

    pub fn expect_delim(&mut self, delim: char) -> ParseResult<()> {
        self.expect_token(Token::Delim(delim))
    }

    pub fn try_delim(&mut self, delim: char) -> bool {
        self.try_parse(|p| p.expect_delim(delim)).is_ok()
    }

    pub fn expect_comma(&mut self) -> ParseResult<()> {
        self.expect_token(Token::Comma)
    }

    pub fn expect_function(&mut self) -> ParseResult<Cow<'i, str>> {
        match self.next_token()? {
            Token::Function(name) => Ok(name),
            _ => Err(ParseError::Unexpected),
        }
    }

    pub fn expect_function_matching(&mut self, name: &str) -> ParseResult<()> {
        match self.next_token()? {
            Token::Function(function) if function.eq_ignore_ascii_case(name) => Ok(()),
            _ => Err(ParseError::Unexpected),
        }
    }
}
