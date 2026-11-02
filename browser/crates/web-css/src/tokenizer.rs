use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Numeric {
    pub value: f64,
    pub is_integer: bool,
    pub has_sign: bool,
}

impl Numeric {
    pub fn int_value(&self) -> Option<i32> {
        self.is_integer
            .then(|| self.value.clamp(i32::MIN as f64, i32::MAX as f64) as i32)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Token<'i> {
    Ident(Cow<'i, str>),
    Function(Cow<'i, str>),
    AtKeyword(Cow<'i, str>),
    Hash(Cow<'i, str>),
    IdHash(Cow<'i, str>),
    String(Cow<'i, str>),
    BadString,
    Url(Cow<'i, str>),
    BadUrl,
    Delim(char),
    Number(Numeric),
    Percentage(Numeric),
    Dimension(Numeric, Cow<'i, str>),
    UnicodeRange(u32, u32),
    Whitespace,
    Cdo,
    Cdc,
    Colon,
    Semicolon,
    Comma,
    IncludeMatch,
    DashMatch,
    PrefixMatch,
    SuffixMatch,
    SubstringMatch,
    Column,
    OpenSquare,
    CloseSquare,
    OpenParen,
    CloseParen,
    OpenCurly,
    CloseCurly,
}

impl Token<'_> {
    pub fn is_ident(&self, name: &str) -> bool {
        matches!(self, Token::Ident(ident) if ident.eq_ignore_ascii_case(name))
    }

    pub fn block_end(&self) -> Option<Token<'static>> {
        match self {
            Token::Function(_) | Token::OpenParen => Some(Token::CloseParen),
            Token::OpenSquare => Some(Token::CloseSquare),
            Token::OpenCurly => Some(Token::CloseCurly),
            _ => None,
        }
    }
}

pub fn preprocess(input: &str) -> Cow<'_, str> {
    if !input.bytes().any(|b| matches!(b, b'\r' | b'\x0c' | b'\0')) {
        return Cow::Borrowed(input);
    }
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                chars.next_if_eq(&'\n');
                out.push('\n');
            }
            '\x0c' => out.push('\n'),
            '\0' => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

#[derive(Clone)]
pub struct Tokenizer<'i> {
    input: &'i str,
    pos: usize,
    unicode_ranges: bool,
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b >= 0x80
}

fn is_ident_char(b: u8) -> bool {
    is_ident_start(b) || b.is_ascii_digit() || b == b'-'
}

fn is_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n')
}

fn is_non_printable(b: u8) -> bool {
    matches!(b, 0..=8 | 0x0b | 0x0e..=0x1f | 0x7f)
}

fn hex_value(b: u8) -> Option<u32> {
    (b as char).to_digit(16)
}

impl<'i> Tokenizer<'i> {
    pub fn new(input: &'i str) -> Self {
        Tokenizer {
            input,
            pos: 0,
            unicode_ranges: false,
        }
    }

    pub fn with_unicode_ranges(input: &'i str) -> Self {
        Tokenizer {
            unicode_ranges: true,
            ..Tokenizer::new(input)
        }
    }

    pub fn input(&self) -> &'i str {
        self.input
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn reset(&mut self, pos: usize) {
        self.pos = pos.min(self.input.len());
    }

    pub fn slice(&self, start: usize, end: usize) -> &'i str {
        self.input.get(start..end).unwrap_or_default()
    }

    fn byte_at(&self, offset: usize) -> Option<u8> {
        self.input.as_bytes().get(self.pos + offset).copied()
    }

    pub fn next_byte(&self) -> Option<u8> {
        self.byte_at(0)
    }

    fn at_eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn rest(&self) -> &'i str {
        self.input.get(self.pos..).unwrap_or_default()
    }

    fn advance(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.input.len());
    }

    fn next_char(&mut self) -> Option<char> {
        let c = self.rest().chars().next()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn valid_escape_at(&self, offset: usize) -> bool {
        self.byte_at(offset) == Some(b'\\') && self.byte_at(offset + 1) != Some(b'\n')
    }

    fn starts_ident_at(&self, offset: usize) -> bool {
        match self.byte_at(offset) {
            Some(b'-') => match self.byte_at(offset + 1) {
                Some(b'-') => true,
                Some(b) if is_ident_start(b) => true,
                _ => self.valid_escape_at(offset + 1),
            },
            Some(b'\\') => self.valid_escape_at(offset),
            Some(b) => is_ident_start(b),
            None => false,
        }
    }

    fn starts_number_at(&self, offset: usize) -> bool {
        let digit = |o: usize| self.byte_at(o).is_some_and(|b| b.is_ascii_digit());
        match self.byte_at(offset) {
            Some(b'+' | b'-') => {
                digit(offset + 1) || (self.byte_at(offset + 1) == Some(b'.') && digit(offset + 2))
            }
            Some(b'.') => digit(offset + 1),
            Some(b) => b.is_ascii_digit(),
            None => false,
        }
    }

    pub fn skip_comments(&mut self) {
        while self.rest().starts_with("/*") {
            match self
                .input
                .get(self.pos + 2..)
                .and_then(|rest| rest.find("*/"))
            {
                Some(end) => self.pos += 2 + end + 2,
                None => self.pos = self.input.len(),
            }
        }
    }

    pub fn next_token(&mut self) -> Option<Token<'i>> {
        self.skip_comments();
        let b = self.next_byte()?;
        let token = match b {
            b' ' | b'\t' | b'\n' => {
                while self.next_byte().is_some_and(is_whitespace) {
                    self.pos += 1;
                }
                Token::Whitespace
            }
            b'"' | b'\'' => {
                self.pos += 1;
                self.consume_string(b)
            }
            b'#' => {
                if self.byte_at(1).is_some_and(is_ident_char) || self.valid_escape_at(1) {
                    let is_id = self.starts_ident_at(1);
                    self.pos += 1;
                    let name = self.consume_ident_sequence();
                    if is_id {
                        Token::IdHash(name)
                    } else {
                        Token::Hash(name)
                    }
                } else {
                    self.delim()
                }
            }
            b'(' => self.single(Token::OpenParen),
            b')' => self.single(Token::CloseParen),
            b',' => self.single(Token::Comma),
            b':' => self.single(Token::Colon),
            b';' => self.single(Token::Semicolon),
            b'[' => self.single(Token::OpenSquare),
            b']' => self.single(Token::CloseSquare),
            b'{' => self.single(Token::OpenCurly),
            b'}' => self.single(Token::CloseCurly),
            b'+' | b'.' if self.starts_number_at(0) => self.consume_numeric(),
            b'-' => {
                if self.starts_number_at(0) {
                    self.consume_numeric()
                } else if self.rest().starts_with("-->") {
                    self.advance(3);
                    Token::Cdc
                } else if self.starts_ident_at(0) {
                    self.consume_ident_like()
                } else {
                    self.delim()
                }
            }
            b'<' if self.rest().starts_with("<!--") => {
                self.advance(4);
                Token::Cdo
            }
            b'@' => {
                if self.starts_ident_at(1) {
                    self.pos += 1;
                    Token::AtKeyword(self.consume_ident_sequence())
                } else {
                    self.delim()
                }
            }
            b'\\' => {
                if self.valid_escape_at(0) {
                    self.consume_ident_like()
                } else {
                    self.delim()
                }
            }
            b'0'..=b'9' => self.consume_numeric(),
            b'~' | b'|' | b'^' | b'$' | b'*' => self.consume_match_or_delim(b),
            b'u' | b'U'
                if self.unicode_ranges
                    && self.byte_at(1) == Some(b'+')
                    && self
                        .byte_at(2)
                        .is_some_and(|b| b == b'?' || b.is_ascii_hexdigit()) =>
            {
                self.advance(2);
                self.consume_unicode_range()
            }
            b if is_ident_start(b) => self.consume_ident_like(),
            _ => self.delim(),
        };
        Some(token)
    }

    fn single(&mut self, token: Token<'i>) -> Token<'i> {
        self.pos += 1;
        token
    }

    fn delim(&mut self) -> Token<'i> {
        Token::Delim(self.next_char().unwrap_or('\u{fffd}'))
    }

    fn consume_match_or_delim(&mut self, b: u8) -> Token<'i> {
        let token = match (b, self.byte_at(1)) {
            (b'~', Some(b'=')) => Token::IncludeMatch,
            (b'|', Some(b'=')) => Token::DashMatch,
            (b'^', Some(b'=')) => Token::PrefixMatch,
            (b'$', Some(b'=')) => Token::SuffixMatch,
            (b'*', Some(b'=')) => Token::SubstringMatch,
            (b'|', Some(b'|')) => Token::Column,
            _ => return self.delim(),
        };
        self.advance(2);
        token
    }

    fn consume_escape(&mut self) -> char {
        let Some(b) = self.next_byte() else {
            return '\u{fffd}';
        };
        if let Some(mut value) = hex_value(b) {
            self.pos += 1;
            let mut digits = 1;
            while digits < 6 {
                match self.next_byte().and_then(hex_value) {
                    Some(digit) => {
                        value = value * 16 + digit;
                        self.pos += 1;
                        digits += 1;
                    }
                    None => break,
                }
            }
            if self.next_byte().is_some_and(is_whitespace) {
                self.pos += 1;
            }
            match char::from_u32(value) {
                Some('\0') | None => '\u{fffd}',
                Some(c) => c,
            }
        } else {
            self.next_char().unwrap_or('\u{fffd}')
        }
    }

    fn consume_ident_sequence(&mut self) -> Cow<'i, str> {
        let start = self.pos;
        while self.next_byte().is_some_and(is_ident_char) {
            self.pos += 1;
        }
        if !self.valid_escape_at(0) {
            return Cow::Borrowed(self.slice(start, self.pos));
        }
        let mut owned = self.slice(start, self.pos).to_owned();
        loop {
            match self.next_byte() {
                Some(b) if is_ident_char(b) => {
                    let run_start = self.pos;
                    while self.next_byte().is_some_and(is_ident_char) {
                        self.pos += 1;
                    }
                    owned.push_str(self.slice(run_start, self.pos));
                }
                Some(b'\\') if self.valid_escape_at(0) => {
                    self.pos += 1;
                    owned.push(self.consume_escape());
                }
                _ => return Cow::Owned(owned),
            }
        }
    }

    fn consume_ident_like(&mut self) -> Token<'i> {
        let name = self.consume_ident_sequence();
        if self.next_byte() != Some(b'(') {
            return Token::Ident(name);
        }
        self.pos += 1;
        if !name.eq_ignore_ascii_case("url") {
            return Token::Function(name);
        }
        let mut offset = 0;
        while self.byte_at(offset).is_some_and(is_whitespace) {
            offset += 1;
        }
        if matches!(self.byte_at(offset), Some(b'"' | b'\'')) {
            if offset > 1 {
                self.advance(offset - 1);
            }
            return Token::Function(name);
        }
        self.advance(offset);
        self.consume_url()
    }

    fn consume_string(&mut self, quote: u8) -> Token<'i> {
        let start = self.pos;
        while let Some(b) = self.next_byte() {
            if b == quote || b == b'\\' || b == b'\n' {
                break;
            }
            self.pos += 1;
        }
        match self.next_byte() {
            None => return Token::String(Cow::Borrowed(self.slice(start, self.pos))),
            Some(b) if b == quote => {
                let value = self.slice(start, self.pos);
                self.pos += 1;
                return Token::String(Cow::Borrowed(value));
            }
            Some(b'\n') => return Token::BadString,
            _ => {}
        }
        let mut owned = self.slice(start, self.pos).to_owned();
        loop {
            match self.next_byte() {
                None => return Token::String(Cow::Owned(owned)),
                Some(b) if b == quote => {
                    self.pos += 1;
                    return Token::String(Cow::Owned(owned));
                }
                Some(b'\n') => return Token::BadString,
                Some(b'\\') => {
                    self.pos += 1;
                    match self.next_byte() {
                        None => {}
                        Some(b'\n') => self.pos += 1,
                        Some(_) => owned.push(self.consume_escape()),
                    }
                }
                Some(_) => {
                    let run_start = self.pos;
                    while self
                        .next_byte()
                        .is_some_and(|b| b != quote && b != b'\\' && b != b'\n')
                    {
                        self.pos += 1;
                    }
                    owned.push_str(self.slice(run_start, self.pos));
                }
            }
        }
    }

    fn consume_url(&mut self) -> Token<'i> {
        let start = self.pos;
        let mut owned: Option<String> = None;
        loop {
            let Some(b) = self.next_byte() else {
                return self.url_value(start, self.pos, owned);
            };
            match b {
                b')' => {
                    let token = self.url_value(start, self.pos, owned);
                    self.pos += 1;
                    return token;
                }
                b' ' | b'\t' | b'\n' => {
                    let end = self.pos;
                    while self.next_byte().is_some_and(is_whitespace) {
                        self.pos += 1;
                    }
                    match self.next_byte() {
                        None => return self.url_value(start, end, owned),
                        Some(b')') => {
                            self.pos += 1;
                            return self.url_value(start, end, owned);
                        }
                        _ => return self.consume_bad_url(),
                    }
                }
                b'"' | b'\'' | b'(' => return self.consume_bad_url(),
                b if is_non_printable(b) => return self.consume_bad_url(),
                b'\\' => {
                    if !self.valid_escape_at(0) {
                        return self.consume_bad_url();
                    }
                    let text = owned.get_or_insert_with(|| self.slice(start, self.pos).to_owned());
                    self.pos += 1;
                    let c = self.consume_escape();
                    text.push(c);
                }
                _ => {
                    let c_start = self.pos;
                    let _ = self.next_char();
                    if let Some(text) = owned.as_mut() {
                        text.push_str(self.slice(c_start, self.pos));
                    }
                }
            }
        }
    }

    fn url_value(&self, start: usize, end: usize, owned: Option<String>) -> Token<'i> {
        Token::Url(match owned {
            Some(text) => Cow::Owned(text),
            None => Cow::Borrowed(self.slice(start, end)),
        })
    }

    fn consume_bad_url(&mut self) -> Token<'i> {
        while let Some(b) = self.next_byte() {
            if b == b')' {
                self.pos += 1;
                break;
            }
            if self.valid_escape_at(0) {
                self.pos += 1;
                self.consume_escape();
            } else {
                let _ = self.next_char();
            }
        }
        Token::BadUrl
    }

    fn consume_number(&mut self) -> Numeric {
        let start = self.pos;
        let has_sign = matches!(self.next_byte(), Some(b'+' | b'-'));
        if has_sign {
            self.pos += 1;
        }
        let skip_digits = |t: &mut Self| {
            while t.next_byte().is_some_and(|b| b.is_ascii_digit()) {
                t.pos += 1;
            }
        };
        skip_digits(self);
        let mut is_integer = true;
        if self.next_byte() == Some(b'.') && self.byte_at(1).is_some_and(|b| b.is_ascii_digit()) {
            is_integer = false;
            self.pos += 1;
            skip_digits(self);
        }
        if matches!(self.next_byte(), Some(b'e' | b'E')) {
            let exponent_digit = match self.byte_at(1) {
                Some(b'+' | b'-') => 2,
                _ => 1,
            };
            if self
                .byte_at(exponent_digit)
                .is_some_and(|b| b.is_ascii_digit())
            {
                is_integer = false;
                self.advance(exponent_digit);
                skip_digits(self);
            }
        }
        let repr = self.slice(start, self.pos);
        let value = repr.parse::<f64>().unwrap_or(0.0).clamp(f64::MIN, f64::MAX);
        Numeric {
            value,
            is_integer,
            has_sign,
        }
    }

    fn consume_numeric(&mut self) -> Token<'i> {
        let number = self.consume_number();
        if self.starts_ident_at(0) {
            Token::Dimension(number, self.consume_ident_sequence())
        } else if self.next_byte() == Some(b'%') {
            self.pos += 1;
            Token::Percentage(number)
        } else {
            Token::Number(number)
        }
    }

    fn consume_unicode_range(&mut self) -> Token<'i> {
        let mut start = 0u32;
        let mut digits = 0;
        while digits < 6 {
            match self.next_byte().and_then(hex_value) {
                Some(digit) => {
                    start = start * 16 + digit;
                    self.pos += 1;
                    digits += 1;
                }
                None => break,
            }
        }
        let mut wildcards = 0;
        while digits + wildcards < 6 && self.next_byte() == Some(b'?') {
            self.pos += 1;
            wildcards += 1;
        }
        if wildcards > 0 {
            let start = start << (4 * wildcards);
            let end = start | ((1u32 << (4 * wildcards)) - 1);
            return Token::UnicodeRange(start, end);
        }
        let mut end = start;
        if self.next_byte() == Some(b'-') && self.byte_at(1).is_some_and(|b| b.is_ascii_hexdigit())
        {
            self.pos += 1;
            end = 0;
            let mut digits = 0;
            while digits < 6 {
                match self.next_byte().and_then(hex_value) {
                    Some(digit) => {
                        end = end * 16 + digit;
                        self.pos += 1;
                        digits += 1;
                    }
                    None => break,
                }
            }
        }
        Token::UnicodeRange(start, end)
    }

    pub fn skip_whitespace(&mut self) {
        loop {
            self.skip_comments();
            if !self.next_byte().is_some_and(is_whitespace) {
                return;
            }
            while self.next_byte().is_some_and(is_whitespace) {
                self.pos += 1;
            }
        }
    }

    pub fn at_end(&self) -> bool {
        self.at_eof()
    }
}
