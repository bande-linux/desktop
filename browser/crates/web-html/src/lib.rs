#![forbid(unsafe_code)]

mod encoding;
mod foreign;
mod input;
mod open_elements;
mod sink;
mod tokenizer;
mod tree_builder;

use encoding_rs::{Decoder, Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};

pub use sink::{Attributes, QuirksMode, TreeSink};
pub use web_atoms::{Atom, AtomTable, Namespace, QualName};

use input::Input;
use tokenizer::{State, Tag, Tokenizer};
use tree_builder::TreeBuilder;

const PRESCAN_BYTES: usize = 1024;

#[derive(Clone, Copy, Default)]
pub struct Options {
    pub scripting: bool,
    pub transport_encoding: Option<&'static Encoding>,
}

pub struct Parser<S: TreeSink> {
    input: Input,
    tokenizer: Tokenizer,
    builder: TreeBuilder<S>,
    options: Options,
    sniffed: Vec<u8>,
    decoder: Option<Decoder>,
    decoded: String,
    encoding: &'static Encoding,
    certain: bool,
    reparse: Option<&'static Encoding>,
}

impl<S: TreeSink> Parser<S> {
    pub fn new(sink: S, options: Options) -> Self {
        Parser {
            input: Input::new(),
            tokenizer: Tokenizer::new(State::Data),
            builder: TreeBuilder::new(sink, options.scripting),
            options,
            sniffed: Vec::new(),
            decoder: None,
            decoded: String::new(),
            encoding: UTF_8,
            certain: false,
            reparse: None,
        }
    }

    pub fn new_fragment(sink: S, context: QualName, options: Options) -> Self {
        let mut parser = Parser::new(sink, options);
        let state = parser.builder.start_fragment(context, &Tag::default());
        parser.tokenizer.state = state;
        parser.certain = true;
        parser
    }

    pub fn sink(&self) -> &S {
        &self.builder.sink
    }

    pub fn encoding(&self) -> &'static Encoding {
        self.encoding
    }

    pub fn reparse_encoding(&self) -> Option<&'static Encoding> {
        self.reparse
    }

    pub fn feed_str(&mut self, text: &str) {
        self.certain = true;
        self.input.push(text);
        self.pump();
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        if self.decoder.is_none() {
            self.sniffed.extend_from_slice(bytes);
            if self.sniffed.len() >= PRESCAN_BYTES {
                self.start_decoding();
            }
            return;
        }
        self.decode(bytes, false);
    }

    pub fn finish(mut self) -> S {
        if self.decoder.is_none() && !self.sniffed.is_empty() {
            self.start_decoding();
        }
        if self.decoder.is_some() {
            self.decode(&[], true);
        }
        self.input.eof = true;
        self.pump();
        self.builder.sink
    }

    fn start_decoding(&mut self) {
        let sniffed = std::mem::take(&mut self.sniffed);
        let sniff = sniff_encoding(&sniffed, self.options.transport_encoding);
        self.certain = sniff.certain;
        self.encoding = sniff.encoding;
        self.decoder = Some(sniff.encoding.new_decoder_without_bom_handling());
        self.decode(sniffed.get(sniff.bom_length..).unwrap_or_default(), false);
    }

    fn decode(&mut self, bytes: &[u8], last: bool) {
        let Some(decoder) = &mut self.decoder else {
            return;
        };
        self.decoded.clear();
        let needed = decoder
            .max_utf8_buffer_length(bytes.len())
            .unwrap_or(bytes.len().saturating_mul(3).saturating_add(16));
        self.decoded.reserve(needed);
        let _ = decoder.decode_to_string(bytes, &mut self.decoded, last);
        let decoded = std::mem::take(&mut self.decoded);
        self.input.push(&decoded);
        self.decoded = decoded;
        self.pump();
    }

    fn pump(&mut self) {
        loop {
            self.tokenizer.cdata_allowed = self.builder.foreign_cdata_allowed();
            let sink = &mut self.builder.sink;
            let Some(token) = self
                .tokenizer
                .next(&mut self.input, &mut |name: &str| sink.intern(name))
            else {
                break;
            };
            self.builder.process(token);
            if let Some(state) = self.builder.tokenizer_state.take() {
                self.tokenizer.state = state;
            }
        }
        if let Some(declared) = self.builder.declared_encoding.take()
            && !self.certain
        {
            self.certain = true;
            if declared != self.encoding && self.encoding != UTF_16BE && self.encoding != UTF_16LE {
                self.reparse = Some(declared);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sniffed {
    pub encoding: &'static Encoding,
    pub certain: bool,
    pub bom_length: usize,
}

pub fn sniff_encoding(head: &[u8], transport: Option<&'static Encoding>) -> Sniffed {
    if let Some((encoding, bom_length)) = Encoding::for_bom(head) {
        return Sniffed {
            encoding,
            certain: true,
            bom_length,
        };
    }
    let head = head.get(..PRESCAN_BYTES).unwrap_or(head);
    Sniffed {
        encoding: transport
            .or_else(|| encoding::prescan(head))
            .unwrap_or(WINDOWS_1252),
        certain: transport.is_some(),
        bom_length: 0,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokenizerState {
    Data,
    Plaintext,
    Rcdata,
    Rawtext,
    ScriptData,
    CdataSection,
}

pub enum Token<'a> {
    Characters(&'a str),
    StartTag {
        name: Atom,
        attributes: Attributes<'a>,
        self_closing: bool,
    },
    EndTag {
        name: Atom,
    },
    Comment(&'a str),
    ProcessingInstruction {
        target: &'a str,
        data: &'a str,
    },
    Doctype {
        name: Option<&'a str>,
        public_id: Option<&'a str>,
        system_id: Option<&'a str>,
        force_quirks: bool,
    },
}

pub fn tokenize(
    text: &str,
    state: TokenizerState,
    last_start_tag: Option<&str>,
    atoms: &mut AtomTable,
    mut each: impl FnMut(Token<'_>, &AtomTable),
) {
    let mut tokenizer = Tokenizer::new(match state {
        TokenizerState::Data => State::Data,
        TokenizerState::Plaintext => State::Plaintext,
        TokenizerState::Rcdata => State::Rcdata,
        TokenizerState::Rawtext => State::Rawtext,
        TokenizerState::ScriptData => State::ScriptData,
        TokenizerState::CdataSection => State::CdataSection,
    });
    if let Some(name) = last_start_tag {
        tokenizer.set_last_start_tag(atoms.intern(name));
    }
    let mut input = Input::new();
    input.push(text);
    input.eof = true;
    while let Some(token) = tokenizer.next(&mut input, &mut |name: &str| atoms.intern(name)) {
        let token = match token {
            tokenizer::Token::Text(text) => Token::Characters(text),
            tokenizer::Token::StartTag(tag) => Token::StartTag {
                name: tag.name,
                attributes: tag.attributes(),
                self_closing: tag.self_closing,
            },
            tokenizer::Token::EndTag(tag) => Token::EndTag { name: tag.name },
            tokenizer::Token::Comment(text) => Token::Comment(text),
            tokenizer::Token::ProcessingInstruction(target, data) => {
                Token::ProcessingInstruction { target, data }
            }
            tokenizer::Token::Doctype(doctype) => Token::Doctype {
                name: doctype.name.as_deref(),
                public_id: doctype.public_id.as_deref(),
                system_id: doctype.system_id.as_deref(),
                force_quirks: doctype.force_quirks,
            },
            tokenizer::Token::Eof => break,
        };
        each(token, atoms);
    }
}
