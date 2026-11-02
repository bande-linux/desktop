use web_atoms::{Atom, QualName};

use crate::input::{Input, Lookahead};
use crate::sink::{Attr, Attributes};

include!(concat!(env!("OUT_DIR"), "/entities.rs"));

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Raw {
    Rcdata,
    Rawtext,
    Script,
    ScriptEscaped,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum State {
    Data,
    Rcdata,
    Rawtext,
    ScriptData,
    Plaintext,
    TagOpen,
    EndTagOpen,
    TagName,
    RawLessThan(Raw),
    RawEndTagOpen(Raw),
    RawEndTagName(Raw),
    ScriptEscapeStart,
    ScriptEscapeStartDash,
    ScriptEscaped,
    ScriptEscapedDash,
    ScriptEscapedDashDash,
    ScriptDoubleEscapeStart,
    ScriptDoubleEscaped,
    ScriptDoubleEscapedDash,
    ScriptDoubleEscapedDashDash,
    ScriptDoubleEscapedLessThan,
    ScriptDoubleEscapeEnd,
    BeforeAttrName,
    AttrName,
    AfterAttrName,
    BeforeAttrValue,
    AttrValueDouble,
    AttrValueSingle,
    AttrValueUnquoted,
    AfterAttrValueQuoted,
    SelfClosingStartTag,
    BogusComment,
    MarkupDeclarationOpen,
    CommentStart,
    CommentStartDash,
    Comment,
    CommentLessThan,
    CommentLessThanBang,
    CommentLessThanBangDash,
    CommentLessThanBangDashDash,
    CommentEndDash,
    CommentEnd,
    CommentEndBang,
    Doctype,
    BeforeDoctypeName,
    DoctypeName,
    AfterDoctypeName,
    AfterDoctypePublicKeyword,
    BeforeDoctypePublicId,
    DoctypePublicId(char),
    AfterDoctypePublicId,
    BetweenDoctypeIds,
    AfterDoctypeSystemKeyword,
    BeforeDoctypeSystemId,
    DoctypeSystemId(char),
    AfterDoctypeSystemId,
    BogusDoctype,
    CdataSection,
    CdataSectionBracket,
    CdataSectionEnd,
    PiOpen,
    PiTarget,
    AfterPiTarget,
    PiData,
    PiQuestionable,
    CharRef,
    NamedCharRef,
    AmbiguousAmpersand,
    NumericCharRef,
    HexCharRefStart,
    HexCharRef,
    DecimalCharRef,
    NumericCharRefEnd,
}

#[derive(Clone)]
pub(crate) struct Tag {
    pub(crate) name: Atom,
    pub(crate) self_closing: bool,
    pub(crate) attrs: Vec<Attr>,
    pub(crate) values: String,
}

impl Default for Tag {
    fn default() -> Self {
        Tag {
            name: Atom::HTML,
            self_closing: false,
            attrs: Vec::new(),
            values: String::new(),
        }
    }
}

impl Tag {
    pub(crate) fn attributes(&self) -> Attributes<'_> {
        Attributes {
            list: &self.attrs,
            values: &self.values,
        }
    }

    pub(crate) fn attribute(&self, name: Atom) -> Option<&str> {
        self.attributes().get(QualName::attribute(name))
    }

    pub(crate) fn same_attributes(&self, other: &Tag) -> bool {
        self.attrs.len() == other.attrs.len()
            && self
                .attributes()
                .iter()
                .all(|(name, value)| other.attributes().get(name) == Some(value))
    }
}

#[derive(Default)]
pub(crate) struct Doctype {
    pub(crate) name: Option<String>,
    pub(crate) public_id: Option<String>,
    pub(crate) system_id: Option<String>,
    pub(crate) force_quirks: bool,
}

pub(crate) enum Token<'a> {
    Text(&'a str),
    StartTag(&'a mut Tag),
    EndTag(&'a mut Tag),
    Comment(&'a str),
    ProcessingInstruction(&'a str, &'a str),
    Doctype(&'a Doctype),
    Eof,
}

#[derive(Clone, Copy)]
enum Ready {
    StartTag,
    EndTag,
    Comment,
    ProcessingInstruction,
    Doctype,
    Eof,
}

pub(crate) struct Tokenizer {
    pub(crate) state: State,
    pub(crate) cdata_allowed: bool,
    return_state: State,
    text: String,
    text_emitted: bool,
    ready: Option<Ready>,
    done: bool,
    tag: Tag,
    end_tag: bool,
    tag_name: String,
    attr_name: String,
    attr_start: usize,
    attr_open: bool,
    attr_duplicate: bool,
    attr_atom: Atom,
    last_start_tag: Option<Atom>,
    temp: String,
    comment: String,
    pi_data: String,
    doctype: Doctype,
    char_code: u32,
}

fn is_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0C' | ' ')
}

fn windows_1252_override(code: u32) -> Option<u32> {
    const TABLE: [u16; 32] = [
        0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039,
        0x0152, 0, 0x017D, 0, 0, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC,
        0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
    ];
    let index = code.checked_sub(0x80)? as usize;
    TABLE
        .get(index)
        .copied()
        .filter(|&mapped| mapped != 0)
        .map(u32::from)
}

impl Tokenizer {
    pub(crate) fn new(state: State) -> Self {
        Tokenizer {
            state,
            cdata_allowed: false,
            return_state: State::Data,
            text: String::new(),
            text_emitted: false,
            ready: None,
            done: false,
            tag: Tag::default(),
            end_tag: false,
            tag_name: String::new(),
            attr_name: String::new(),
            attr_start: 0,
            attr_open: false,
            attr_duplicate: false,
            attr_atom: Atom::HTML,
            last_start_tag: None,
            temp: String::new(),
            comment: String::new(),
            pi_data: String::new(),
            doctype: Doctype::default(),
            char_code: 0,
        }
    }

    pub(crate) fn set_last_start_tag(&mut self, name: Atom) {
        self.last_start_tag = Some(name);
    }

    pub(crate) fn next<I: FnMut(&str) -> Atom>(
        &mut self,
        input: &mut Input,
        intern: &mut I,
    ) -> Option<Token<'_>> {
        if self.text_emitted {
            self.text.clear();
            self.text_emitted = false;
        }
        if self.ready.is_none() && !self.done {
            self.run(input, intern);
        }
        if !self.text.is_empty() {
            self.text_emitted = true;
            return Some(Token::Text(&self.text));
        }
        Some(match self.ready.take()? {
            Ready::StartTag => Token::StartTag(&mut self.tag),
            Ready::EndTag => Token::EndTag(&mut self.tag),
            Ready::Comment => Token::Comment(&self.comment),
            Ready::ProcessingInstruction => Token::ProcessingInstruction(&self.temp, &self.pi_data),
            Ready::Doctype => Token::Doctype(&self.doctype),
            Ready::Eof => Token::Eof,
        })
    }

    fn emit(&mut self, ready: Ready) {
        if matches!(ready, Ready::Eof) {
            self.done = true;
        }
        self.ready = Some(ready);
    }

    fn new_tag(&mut self, end: bool) {
        self.end_tag = end;
        self.tag_name.clear();
        self.tag.self_closing = false;
        self.tag.attrs.clear();
        self.tag.values.clear();
        self.attr_open = false;
    }

    fn start_attr(&mut self) {
        self.finish_attr();
        self.attr_name.clear();
        self.attr_open = true;
        self.attr_duplicate = false;
        self.attr_start = self.tag.values.len();
    }

    fn name_attr<I: FnMut(&str) -> Atom>(&mut self, intern: &mut I) {
        let atom = intern(&self.attr_name);
        self.attr_atom = atom;
        self.attr_duplicate = self.tag.attrs.iter().any(|attr| attr.name.local == atom);
    }

    fn finish_attr(&mut self) {
        if !self.attr_open {
            return;
        }
        self.attr_open = false;
        if self.attr_duplicate {
            self.tag.values.truncate(self.attr_start);
            return;
        }
        let (Ok(start), Ok(end)) = (
            u32::try_from(self.attr_start),
            u32::try_from(self.tag.values.len()),
        ) else {
            return;
        };
        self.tag.attrs.push(Attr {
            name: QualName::attribute(self.attr_atom),
            start,
            end,
        });
    }

    fn emit_tag<I: FnMut(&str) -> Atom>(&mut self, intern: &mut I) {
        self.finish_attr();
        self.tag.name = intern(&self.tag_name);
        self.state = State::Data;
        if self.end_tag {
            self.emit(Ready::EndTag);
        } else {
            self.last_start_tag = Some(self.tag.name);
            self.emit(Ready::StartTag);
        }
    }

    fn appropriate_end_tag(&self) -> bool {
        self.last_start_tag.is_some() && Atom::from_static(&self.tag_name) == self.last_start_tag
    }

    fn in_attribute(&self) -> bool {
        matches!(
            self.return_state,
            State::AttrValueDouble | State::AttrValueSingle | State::AttrValueUnquoted
        )
    }

    fn flush_char_ref(&mut self) {
        if self.in_attribute() {
            self.tag.values.push_str(&self.temp);
        } else {
            self.text.push_str(&self.temp);
        }
    }

    fn push_char_ref(&mut self, c: char) {
        if self.in_attribute() {
            self.tag.values.push(c);
        } else {
            self.text.push(c);
        }
    }

    fn take_run(&mut self, input: &mut Input, stops: &[u8]) -> bool {
        let rest = input.rest();
        let n = rest
            .bytes()
            .position(|b| stops.contains(&b))
            .unwrap_or(rest.len());
        if n == 0 {
            return false;
        }
        self.text.push_str(&rest[..n]);
        input.advance(n);
        true
    }

    fn take_value_run(&mut self, input: &mut Input, stops: &[u8]) -> bool {
        let rest = input.rest();
        let n = rest
            .bytes()
            .position(|b| stops.contains(&b))
            .unwrap_or(rest.len());
        if n == 0 {
            return false;
        }
        self.tag.values.push_str(&rest[..n]);
        input.advance(n);
        true
    }

    fn bogus_comment_from_temp(&mut self) {
        self.comment.clear();
        self.comment.push('?');
        self.comment.push_str(&self.temp);
        self.state = State::BogusComment;
    }

    fn run<I: FnMut(&str) -> Atom>(&mut self, input: &mut Input, intern: &mut I) {
        while self.ready.is_none() {
            match self.state {
                State::Data => {
                    if self.take_run(input, b"&<\0") {
                        continue;
                    }
                }
                State::Rcdata => {
                    if self.take_run(input, b"&<\0") {
                        continue;
                    }
                }
                State::Rawtext | State::ScriptData => {
                    if self.take_run(input, b"<\0") {
                        continue;
                    }
                }
                State::Plaintext => {
                    if self.take_run(input, b"\0") {
                        continue;
                    }
                }
                State::AttrValueDouble => {
                    if self.take_value_run(input, b"\"&\0") {
                        continue;
                    }
                }
                State::AttrValueSingle => {
                    if self.take_value_run(input, b"'&\0") {
                        continue;
                    }
                }
                State::NamedCharRef => {
                    if !self.named_char_ref(input) {
                        return;
                    }
                    continue;
                }
                State::MarkupDeclarationOpen => {
                    if !self.markup_declaration_open(input) {
                        return;
                    }
                    continue;
                }
                State::AfterDoctypeName => {
                    if !self.after_doctype_name(input) {
                        return;
                    }
                    continue;
                }
                State::NumericCharRefEnd => {
                    self.numeric_char_ref_end();
                    continue;
                }
                _ => {}
            }
            let c = match input.next_char() {
                Some(c) => Some(c),
                None if input.eof => None,
                None => return,
            };
            self.step(c, input, intern);
            if c.is_none() && !matches!(self.ready, None | Some(Ready::Eof)) {
                self.state = State::Data;
            }
        }
    }

    fn step<I: FnMut(&str) -> Atom>(&mut self, c: Option<char>, input: &mut Input, intern: &mut I) {
        let reconsume = |input: &mut Input| {
            if let Some(c) = c {
                input.unread(c);
            }
        };
        match self.state {
            State::Data => match c {
                Some('&') => {
                    self.return_state = State::Data;
                    self.state = State::CharRef;
                }
                Some('<') => self.state = State::TagOpen,
                Some(c) => self.text.push(c),
                None => self.emit(Ready::Eof),
            },
            State::Rcdata => match c {
                Some('&') => {
                    self.return_state = State::Rcdata;
                    self.state = State::CharRef;
                }
                Some('<') => self.state = State::RawLessThan(Raw::Rcdata),
                Some('\0') => self.text.push('\u{FFFD}'),
                Some(c) => self.text.push(c),
                None => self.emit(Ready::Eof),
            },
            State::Rawtext | State::ScriptData | State::Plaintext => match c {
                Some('<') if self.state == State::Rawtext => {
                    self.state = State::RawLessThan(Raw::Rawtext)
                }
                Some('<') if self.state == State::ScriptData => {
                    self.state = State::RawLessThan(Raw::Script)
                }
                Some('\0') => self.text.push('\u{FFFD}'),
                Some(c) => self.text.push(c),
                None => self.emit(Ready::Eof),
            },
            State::TagOpen => match c {
                Some('!') => self.state = State::MarkupDeclarationOpen,
                Some('/') => self.state = State::EndTagOpen,
                Some(c) if c.is_ascii_alphabetic() => {
                    self.new_tag(false);
                    reconsume(input);
                    self.state = State::TagName;
                }
                Some('?') => {
                    self.temp.clear();
                    self.state = State::PiOpen;
                }
                None => {
                    self.text.push('<');
                    self.emit(Ready::Eof);
                }
                Some(_) => {
                    self.text.push('<');
                    reconsume(input);
                    self.state = State::Data;
                }
            },
            State::EndTagOpen => match c {
                Some(c) if c.is_ascii_alphabetic() => {
                    self.new_tag(true);
                    reconsume(input);
                    self.state = State::TagName;
                }
                Some('>') => self.state = State::Data,
                None => {
                    self.text.push_str("</");
                    self.emit(Ready::Eof);
                }
                Some(_) => {
                    self.comment.clear();
                    reconsume(input);
                    self.state = State::BogusComment;
                }
            },
            State::TagName => match c {
                Some(c) if is_space(c) => self.state = State::BeforeAttrName,
                Some('/') => self.state = State::SelfClosingStartTag,
                Some('>') => self.emit_tag(intern),
                Some('\0') => self.tag_name.push('\u{FFFD}'),
                Some(c) => self.tag_name.push(c.to_ascii_lowercase()),
                None => self.emit(Ready::Eof),
            },
            State::RawLessThan(raw) => match (raw, c) {
                (_, Some('/')) => {
                    self.temp.clear();
                    self.state = State::RawEndTagOpen(raw);
                }
                (Raw::Script, Some('!')) => {
                    self.text.push_str("<!");
                    self.state = State::ScriptEscapeStart;
                }
                (Raw::ScriptEscaped, Some(c)) if c.is_ascii_alphabetic() => {
                    self.temp.clear();
                    self.text.push('<');
                    reconsume(input);
                    self.state = State::ScriptDoubleEscapeStart;
                }
                _ => {
                    self.text.push('<');
                    reconsume(input);
                    self.state = raw_state(raw);
                }
            },
            State::RawEndTagOpen(raw) => match c {
                Some(c) if c.is_ascii_alphabetic() => {
                    self.new_tag(true);
                    reconsume(input);
                    self.state = State::RawEndTagName(raw);
                }
                _ => {
                    self.text.push_str("</");
                    reconsume(input);
                    self.state = raw_state(raw);
                }
            },
            State::RawEndTagName(raw) => match c {
                Some(c) if is_space(c) && self.appropriate_end_tag() => {
                    self.state = State::BeforeAttrName
                }
                Some('/') if self.appropriate_end_tag() => self.state = State::SelfClosingStartTag,
                Some('>') if self.appropriate_end_tag() => self.emit_tag(intern),
                Some(c) if c.is_ascii_alphabetic() => {
                    self.tag_name.push(c.to_ascii_lowercase());
                    self.temp.push(c);
                }
                _ => {
                    self.text.push_str("</");
                    self.text.push_str(&self.temp);
                    reconsume(input);
                    self.state = raw_state(raw);
                }
            },
            State::ScriptEscapeStart | State::ScriptEscapeStartDash => match c {
                Some('-') => {
                    self.text.push('-');
                    self.state = if self.state == State::ScriptEscapeStart {
                        State::ScriptEscapeStartDash
                    } else {
                        State::ScriptEscapedDashDash
                    };
                }
                _ => {
                    reconsume(input);
                    self.state = State::ScriptData;
                }
            },
            State::ScriptEscaped => match c {
                Some('-') => {
                    self.text.push('-');
                    self.state = State::ScriptEscapedDash;
                }
                Some('<') => self.state = State::RawLessThan(Raw::ScriptEscaped),
                Some('\0') => self.text.push('\u{FFFD}'),
                Some(c) => self.text.push(c),
                None => self.emit(Ready::Eof),
            },
            State::ScriptEscapedDash | State::ScriptEscapedDashDash => match c {
                Some('-') => {
                    self.text.push('-');
                    self.state = State::ScriptEscapedDashDash;
                }
                Some('<') => self.state = State::RawLessThan(Raw::ScriptEscaped),
                Some('>') if self.state == State::ScriptEscapedDashDash => {
                    self.text.push('>');
                    self.state = State::ScriptData;
                }
                Some('\0') => {
                    self.text.push('\u{FFFD}');
                    self.state = State::ScriptEscaped;
                }
                Some(c) => {
                    self.text.push(c);
                    self.state = State::ScriptEscaped;
                }
                None => self.emit(Ready::Eof),
            },
            State::ScriptDoubleEscapeStart | State::ScriptDoubleEscapeEnd => match c {
                Some(c) if is_space(c) || c == '/' || c == '>' => {
                    self.text.push(c);
                    let escaped = self.temp == "script";
                    self.state = match (self.state, escaped) {
                        (State::ScriptDoubleEscapeStart, true) => State::ScriptDoubleEscaped,
                        (State::ScriptDoubleEscapeStart, false) => State::ScriptEscaped,
                        (_, true) => State::ScriptEscaped,
                        (_, false) => State::ScriptDoubleEscaped,
                    };
                }
                Some(c) if c.is_ascii_alphabetic() => {
                    self.temp.push(c.to_ascii_lowercase());
                    self.text.push(c);
                }
                _ => {
                    reconsume(input);
                    self.state = if self.state == State::ScriptDoubleEscapeStart {
                        State::ScriptEscaped
                    } else {
                        State::ScriptDoubleEscaped
                    };
                }
            },
            State::ScriptDoubleEscaped
            | State::ScriptDoubleEscapedDash
            | State::ScriptDoubleEscapedDashDash => match c {
                Some('-') => {
                    self.text.push('-');
                    self.state = if self.state == State::ScriptDoubleEscaped {
                        State::ScriptDoubleEscapedDash
                    } else {
                        State::ScriptDoubleEscapedDashDash
                    };
                }
                Some('<') => {
                    self.text.push('<');
                    self.state = State::ScriptDoubleEscapedLessThan;
                }
                Some('>') if self.state == State::ScriptDoubleEscapedDashDash => {
                    self.text.push('>');
                    self.state = State::ScriptData;
                }
                Some('\0') => {
                    self.text.push('\u{FFFD}');
                    self.state = State::ScriptDoubleEscaped;
                }
                Some(c) => {
                    self.text.push(c);
                    self.state = State::ScriptDoubleEscaped;
                }
                None => self.emit(Ready::Eof),
            },
            State::ScriptDoubleEscapedLessThan => match c {
                Some('/') => {
                    self.temp.clear();
                    self.text.push('/');
                    self.state = State::ScriptDoubleEscapeEnd;
                }
                _ => {
                    reconsume(input);
                    self.state = State::ScriptDoubleEscaped;
                }
            },
            State::BeforeAttrName => match c {
                Some(c) if is_space(c) => {}
                Some('/') | Some('>') | None => {
                    reconsume(input);
                    self.state = State::AfterAttrName;
                }
                Some('=') => {
                    self.start_attr();
                    self.attr_name.push('=');
                    self.state = State::AttrName;
                }
                Some(_) => {
                    self.start_attr();
                    reconsume(input);
                    self.state = State::AttrName;
                }
            },
            State::AttrName => match c {
                Some(c) if is_space(c) || c == '/' || c == '>' => {
                    self.name_attr(intern);
                    reconsume(input);
                    self.state = State::AfterAttrName;
                }
                None => {
                    self.name_attr(intern);
                    self.state = State::AfterAttrName;
                }
                Some('=') => {
                    self.name_attr(intern);
                    self.state = State::BeforeAttrValue;
                }
                Some('\0') => self.attr_name.push('\u{FFFD}'),
                Some(c) => self.attr_name.push(c.to_ascii_lowercase()),
            },
            State::AfterAttrName => match c {
                Some(c) if is_space(c) => {}
                Some('/') => self.state = State::SelfClosingStartTag,
                Some('=') => self.state = State::BeforeAttrValue,
                Some('>') => self.emit_tag(intern),
                None => self.emit(Ready::Eof),
                Some(_) => {
                    self.start_attr();
                    reconsume(input);
                    self.state = State::AttrName;
                }
            },
            State::BeforeAttrValue => match c {
                Some(c) if is_space(c) => {}
                Some('"') => self.state = State::AttrValueDouble,
                Some('\'') => self.state = State::AttrValueSingle,
                Some('>') => self.emit_tag(intern),
                _ => {
                    reconsume(input);
                    self.state = State::AttrValueUnquoted;
                }
            },
            State::AttrValueDouble | State::AttrValueSingle => match c {
                Some('"') if self.state == State::AttrValueDouble => {
                    self.state = State::AfterAttrValueQuoted
                }
                Some('\'') if self.state == State::AttrValueSingle => {
                    self.state = State::AfterAttrValueQuoted
                }
                Some('&') => {
                    self.return_state = self.state;
                    self.state = State::CharRef;
                }
                Some('\0') => self.tag.values.push('\u{FFFD}'),
                Some(c) => self.tag.values.push(c),
                None => self.emit(Ready::Eof),
            },
            State::AttrValueUnquoted => match c {
                Some(c) if is_space(c) => self.state = State::BeforeAttrName,
                Some('&') => {
                    self.return_state = State::AttrValueUnquoted;
                    self.state = State::CharRef;
                }
                Some('>') => self.emit_tag(intern),
                Some('\0') => self.tag.values.push('\u{FFFD}'),
                Some(c) => self.tag.values.push(c),
                None => self.emit(Ready::Eof),
            },
            State::AfterAttrValueQuoted => match c {
                Some(c) if is_space(c) => self.state = State::BeforeAttrName,
                Some('/') => self.state = State::SelfClosingStartTag,
                Some('>') => self.emit_tag(intern),
                None => self.emit(Ready::Eof),
                Some(_) => {
                    reconsume(input);
                    self.state = State::BeforeAttrName;
                }
            },
            State::SelfClosingStartTag => match c {
                Some('>') => {
                    self.tag.self_closing = true;
                    self.emit_tag(intern);
                }
                None => self.emit(Ready::Eof),
                Some(_) => {
                    reconsume(input);
                    self.state = State::BeforeAttrName;
                }
            },
            State::BogusComment => match c {
                Some('>') => {
                    self.state = State::Data;
                    self.emit(Ready::Comment);
                }
                None => self.emit(Ready::Comment),
                Some('\0') => self.comment.push('\u{FFFD}'),
                Some(c) => self.comment.push(c),
            },
            State::CommentStart => match c {
                Some('-') => self.state = State::CommentStartDash,
                Some('>') => {
                    self.state = State::Data;
                    self.emit(Ready::Comment);
                }
                _ => {
                    reconsume(input);
                    self.state = State::Comment;
                }
            },
            State::CommentStartDash => match c {
                Some('-') => self.state = State::CommentEnd,
                Some('>') => {
                    self.state = State::Data;
                    self.emit(Ready::Comment);
                }
                None => self.emit(Ready::Comment),
                Some(_) => {
                    self.comment.push('-');
                    reconsume(input);
                    self.state = State::Comment;
                }
            },
            State::Comment => match c {
                Some('<') => {
                    self.comment.push('<');
                    self.state = State::CommentLessThan;
                }
                Some('-') => self.state = State::CommentEndDash,
                Some('\0') => self.comment.push('\u{FFFD}'),
                Some(c) => self.comment.push(c),
                None => self.emit(Ready::Comment),
            },
            State::CommentLessThan => match c {
                Some('!') => {
                    self.comment.push('!');
                    self.state = State::CommentLessThanBang;
                }
                Some('<') => self.comment.push('<'),
                _ => {
                    reconsume(input);
                    self.state = State::Comment;
                }
            },
            State::CommentLessThanBang => match c {
                Some('-') => self.state = State::CommentLessThanBangDash,
                _ => {
                    reconsume(input);
                    self.state = State::Comment;
                }
            },
            State::CommentLessThanBangDash => match c {
                Some('-') => self.state = State::CommentLessThanBangDashDash,
                _ => {
                    reconsume(input);
                    self.state = State::CommentEndDash;
                }
            },
            State::CommentLessThanBangDashDash => {
                reconsume(input);
                self.state = State::CommentEnd;
            }
            State::CommentEndDash => match c {
                Some('-') => self.state = State::CommentEnd,
                None => self.emit(Ready::Comment),
                Some(_) => {
                    self.comment.push('-');
                    reconsume(input);
                    self.state = State::Comment;
                }
            },
            State::CommentEnd => match c {
                Some('>') => {
                    self.state = State::Data;
                    self.emit(Ready::Comment);
                }
                Some('!') => self.state = State::CommentEndBang,
                Some('-') => self.comment.push('-'),
                None => self.emit(Ready::Comment),
                Some(_) => {
                    self.comment.push_str("--");
                    reconsume(input);
                    self.state = State::Comment;
                }
            },
            State::CommentEndBang => match c {
                Some('-') => {
                    self.comment.push_str("--!");
                    self.state = State::CommentEndDash;
                }
                Some('>') => {
                    self.state = State::Data;
                    self.emit(Ready::Comment);
                }
                None => self.emit(Ready::Comment),
                Some(_) => {
                    self.comment.push_str("--!");
                    reconsume(input);
                    self.state = State::Comment;
                }
            },
            State::Doctype => match c {
                Some(c) if is_space(c) => self.state = State::BeforeDoctypeName,
                None => {
                    self.doctype = Doctype {
                        force_quirks: true,
                        ..Doctype::default()
                    };
                    self.emit(Ready::Doctype);
                }
                Some(_) => {
                    reconsume(input);
                    self.state = State::BeforeDoctypeName;
                }
            },
            State::BeforeDoctypeName => match c {
                Some(c) if is_space(c) => {}
                Some('>') => {
                    self.doctype = Doctype {
                        force_quirks: true,
                        ..Doctype::default()
                    };
                    self.state = State::Data;
                    self.emit(Ready::Doctype);
                }
                None => {
                    self.doctype = Doctype {
                        force_quirks: true,
                        ..Doctype::default()
                    };
                    self.emit(Ready::Doctype);
                }
                Some(c) => {
                    let c = if c == '\0' {
                        '\u{FFFD}'
                    } else {
                        c.to_ascii_lowercase()
                    };
                    self.doctype = Doctype {
                        name: Some(c.to_string()),
                        ..Doctype::default()
                    };
                    self.state = State::DoctypeName;
                }
            },
            State::DoctypeName => match c {
                Some(c) if is_space(c) => self.state = State::AfterDoctypeName,
                Some('>') => {
                    self.state = State::Data;
                    self.emit(Ready::Doctype);
                }
                None => {
                    self.doctype.force_quirks = true;
                    self.emit(Ready::Doctype);
                }
                Some(c) => {
                    let c = if c == '\0' {
                        '\u{FFFD}'
                    } else {
                        c.to_ascii_lowercase()
                    };
                    if let Some(name) = &mut self.doctype.name {
                        name.push(c);
                    }
                }
            },
            State::AfterDoctypePublicKeyword | State::AfterDoctypeSystemKeyword => {
                let public = self.state == State::AfterDoctypePublicKeyword;
                match c {
                    Some(c) if is_space(c) => {
                        self.state = if public {
                            State::BeforeDoctypePublicId
                        } else {
                            State::BeforeDoctypeSystemId
                        }
                    }
                    Some(q @ ('"' | '\'')) => self.start_doctype_id(public, q),
                    _ => self.doctype_missing_id(c, input),
                }
            }
            State::BeforeDoctypePublicId | State::BeforeDoctypeSystemId => {
                let public = self.state == State::BeforeDoctypePublicId;
                match c {
                    Some(c) if is_space(c) => {}
                    Some(q @ ('"' | '\'')) => self.start_doctype_id(public, q),
                    _ => self.doctype_missing_id(c, input),
                }
            }
            State::DoctypePublicId(quote) | State::DoctypeSystemId(quote) => {
                let public = matches!(self.state, State::DoctypePublicId(_));
                match c {
                    Some(c) if c == quote => {
                        self.state = if public {
                            State::AfterDoctypePublicId
                        } else {
                            State::AfterDoctypeSystemId
                        }
                    }
                    Some('>') => {
                        self.doctype.force_quirks = true;
                        self.state = State::Data;
                        self.emit(Ready::Doctype);
                    }
                    None => {
                        self.doctype.force_quirks = true;
                        self.emit(Ready::Doctype);
                    }
                    Some(c) => {
                        let id = if public {
                            &mut self.doctype.public_id
                        } else {
                            &mut self.doctype.system_id
                        };
                        if let Some(id) = id {
                            id.push(if c == '\0' { '\u{FFFD}' } else { c });
                        }
                    }
                }
            }
            State::AfterDoctypePublicId | State::BetweenDoctypeIds => match c {
                Some(c) if is_space(c) => self.state = State::BetweenDoctypeIds,
                Some('>') => {
                    self.state = State::Data;
                    self.emit(Ready::Doctype);
                }
                Some(q @ ('"' | '\'')) => self.start_doctype_id(false, q),
                None => {
                    self.doctype.force_quirks = true;
                    self.emit(Ready::Doctype);
                }
                Some(_) => {
                    self.doctype.force_quirks = true;
                    reconsume(input);
                    self.state = State::BogusDoctype;
                }
            },
            State::AfterDoctypeSystemId => match c {
                Some(c) if is_space(c) => {}
                Some('>') => {
                    self.state = State::Data;
                    self.emit(Ready::Doctype);
                }
                None => {
                    self.doctype.force_quirks = true;
                    self.emit(Ready::Doctype);
                }
                Some(_) => {
                    reconsume(input);
                    self.state = State::BogusDoctype;
                }
            },
            State::BogusDoctype => match c {
                Some('>') => {
                    self.state = State::Data;
                    self.emit(Ready::Doctype);
                }
                None => self.emit(Ready::Doctype),
                Some(_) => {}
            },
            State::CdataSection => match c {
                Some(']') => self.state = State::CdataSectionBracket,
                Some(c) => self.text.push(c),
                None => self.emit(Ready::Eof),
            },
            State::CdataSectionBracket => match c {
                Some(']') => self.state = State::CdataSectionEnd,
                _ => {
                    self.text.push(']');
                    reconsume(input);
                    self.state = State::CdataSection;
                }
            },
            State::CdataSectionEnd => match c {
                Some(']') => self.text.push(']'),
                Some('>') => self.state = State::Data,
                _ => {
                    self.text.push_str("]]");
                    reconsume(input);
                    self.state = State::CdataSection;
                }
            },
            State::PiOpen => match c {
                Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                    reconsume(input);
                    self.state = State::PiTarget;
                }
                None => self.emit(Ready::Eof),
                Some(_) => {
                    reconsume(input);
                    self.bogus_comment_from_temp();
                }
            },
            State::PiTarget => match c {
                Some(c) if is_space(c) || c == '?' || c == '>' => {
                    reconsume(input);
                    if self.temp.eq_ignore_ascii_case("xml")
                        || self.temp.eq_ignore_ascii_case("xml-stylesheet")
                    {
                        self.bogus_comment_from_temp();
                    } else {
                        self.pi_data.clear();
                        self.state = State::AfterPiTarget;
                    }
                }
                Some(c) if c.is_ascii_alphanumeric() || c == '-' || c == '_' => self.temp.push(c),
                None => self.emit(Ready::Eof),
                Some(_) => {
                    reconsume(input);
                    self.bogus_comment_from_temp();
                }
            },
            State::AfterPiTarget => match c {
                Some(c) if is_space(c) => {}
                _ => {
                    reconsume(input);
                    self.state = State::PiData;
                }
            },
            State::PiData | State::PiQuestionable => match c {
                Some('>') => {
                    self.state = State::Data;
                    self.emit(Ready::ProcessingInstruction);
                }
                None => self.emit(Ready::Eof),
                Some(_) if self.state == State::PiQuestionable => {
                    self.pi_data.push('?');
                    reconsume(input);
                    self.state = State::PiData;
                }
                Some('?') => self.state = State::PiQuestionable,
                Some(c) => self.pi_data.push(c),
            },
            State::CharRef => {
                self.temp.clear();
                self.temp.push('&');
                match c {
                    Some(c) if c.is_ascii_alphanumeric() => {
                        reconsume(input);
                        self.state = State::NamedCharRef;
                    }
                    Some('#') => {
                        self.temp.push('#');
                        self.state = State::NumericCharRef;
                    }
                    _ => {
                        self.flush_char_ref();
                        reconsume(input);
                        self.state = self.return_state;
                    }
                }
            }
            State::AmbiguousAmpersand => match c {
                Some(c) if c.is_ascii_alphanumeric() => self.push_char_ref(c),
                _ => {
                    reconsume(input);
                    self.state = self.return_state;
                }
            },
            State::NumericCharRef => {
                self.char_code = 0;
                match c {
                    Some(c @ ('x' | 'X')) => {
                        self.temp.push(c);
                        self.state = State::HexCharRefStart;
                    }
                    Some(c) if c.is_ascii_digit() => {
                        reconsume(input);
                        self.state = State::DecimalCharRef;
                    }
                    _ => {
                        self.flush_char_ref();
                        reconsume(input);
                        self.state = self.return_state;
                    }
                }
            }
            State::HexCharRefStart => match c {
                Some(c) if c.is_ascii_hexdigit() => {
                    reconsume(input);
                    self.state = State::HexCharRef;
                }
                _ => {
                    self.flush_char_ref();
                    reconsume(input);
                    self.state = self.return_state;
                }
            },
            State::HexCharRef | State::DecimalCharRef => {
                let radix = if self.state == State::HexCharRef {
                    16
                } else {
                    10
                };
                match c {
                    Some(c) if c.is_digit(radix) => {
                        let digit = c.to_digit(radix).unwrap_or(0);
                        self.char_code = self
                            .char_code
                            .saturating_mul(radix)
                            .saturating_add(digit)
                            .min(0x11_0000);
                    }
                    Some(';') => self.state = State::NumericCharRefEnd,
                    _ => {
                        reconsume(input);
                        self.state = State::NumericCharRefEnd;
                    }
                }
            }
            State::NamedCharRef
            | State::MarkupDeclarationOpen
            | State::AfterDoctypeName
            | State::NumericCharRefEnd => {
                reconsume(input);
            }
        }
    }

    fn start_doctype_id(&mut self, public: bool, quote: char) {
        if public {
            self.doctype.public_id = Some(String::new());
            self.state = State::DoctypePublicId(quote);
        } else {
            self.doctype.system_id = Some(String::new());
            self.state = State::DoctypeSystemId(quote);
        }
    }

    fn doctype_missing_id(&mut self, c: Option<char>, input: &mut Input) {
        self.doctype.force_quirks = true;
        match c {
            Some('>') => {
                self.state = State::Data;
                self.emit(Ready::Doctype);
            }
            None => self.emit(Ready::Doctype),
            Some(c) => {
                input.unread(c);
                self.state = State::BogusDoctype;
            }
        }
    }

    fn numeric_char_ref_end(&mut self) {
        let mut code = self.char_code;
        if code == 0 || code > 0x10_FFFF || (0xD800..=0xDFFF).contains(&code) {
            code = 0xFFFD;
        } else if let Some(mapped) = windows_1252_override(code) {
            code = mapped;
        }
        let c = char::from_u32(code).unwrap_or('\u{FFFD}');
        self.push_char_ref(c);
        self.state = self.return_state;
    }

    fn named_char_ref(&mut self, input: &mut Input) -> bool {
        let rest = input.rest().as_bytes();
        let (mut lo, mut hi) = (0, ENTITIES.len());
        let mut best = None;
        let mut i = 0;
        loop {
            let Some(&b) = rest.get(i) else {
                if !input.eof {
                    return false;
                }
                break;
            };
            let Some(slice) = ENTITIES.get(lo..hi) else {
                break;
            };
            let at = |name: &str| name.as_bytes().get(i).copied();
            let start = slice.partition_point(|e| at(e.0).is_none_or(|x| x < b));
            let end = slice.partition_point(|e| at(e.0).is_none_or(|x| x <= b));
            hi = lo + end;
            lo += start;
            if lo >= hi {
                break;
            }
            i += 1;
            if ENTITIES.get(lo).is_some_and(|e| e.0.len() == i) {
                best = Some(lo);
            }
        }
        let Some(&(name, first, second)) = best.and_then(|index| ENTITIES.get(index)) else {
            self.flush_char_ref();
            self.state = State::AmbiguousAmpersand;
            return true;
        };
        let next = rest.get(name.len()).copied();
        let historical = self.in_attribute()
            && !name.ends_with(';')
            && next.is_some_and(|b| b == b'=' || b.is_ascii_alphanumeric());
        if historical {
            self.temp.push_str(name);
            self.flush_char_ref();
        } else {
            self.push_char_ref(first);
            if second != '\0' {
                self.push_char_ref(second);
            }
        }
        input.advance(name.len());
        self.state = self.return_state;
        true
    }

    fn markup_declaration_open(&mut self, input: &mut Input) -> bool {
        let mut need_more = false;
        for (word, ignore_case) in [("--", false), ("DOCTYPE", true), ("[CDATA[", false)] {
            match input.lookahead(word, ignore_case) {
                Lookahead::Match => {
                    input.advance(word.len());
                    self.comment.clear();
                    self.state = match word {
                        "--" => State::CommentStart,
                        "DOCTYPE" => State::Doctype,
                        _ if self.cdata_allowed => State::CdataSection,
                        _ => {
                            self.comment.push_str("[CDATA[");
                            State::BogusComment
                        }
                    };
                    return true;
                }
                Lookahead::NeedMore => need_more = true,
                Lookahead::Mismatch => {}
            }
        }
        if need_more {
            return false;
        }
        self.comment.clear();
        self.state = State::BogusComment;
        true
    }

    fn after_doctype_name(&mut self, input: &mut Input) -> bool {
        let rest = input.rest();
        let Some(c) = rest.chars().next() else {
            if !input.eof {
                return false;
            }
            self.doctype.force_quirks = true;
            self.state = State::Data;
            self.emit(Ready::Doctype);
            return true;
        };
        if is_space(c) {
            input.advance(1);
            return true;
        }
        if c == '>' {
            input.advance(1);
            self.state = State::Data;
            self.emit(Ready::Doctype);
            return true;
        }
        for (word, state) in [
            ("PUBLIC", State::AfterDoctypePublicKeyword),
            ("SYSTEM", State::AfterDoctypeSystemKeyword),
        ] {
            match input.lookahead(word, true) {
                Lookahead::Match => {
                    input.advance(word.len());
                    self.state = state;
                    return true;
                }
                Lookahead::NeedMore => return false,
                Lookahead::Mismatch => {}
            }
        }
        self.doctype.force_quirks = true;
        self.state = State::BogusDoctype;
        true
    }
}

fn raw_state(raw: Raw) -> State {
    match raw {
        Raw::Rcdata => State::Rcdata,
        Raw::Rawtext => State::Rawtext,
        Raw::Script => State::ScriptData,
        Raw::ScriptEscaped => State::ScriptEscaped,
    }
}
