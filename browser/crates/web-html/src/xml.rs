use std::collections::{HashMap, HashSet};

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};
use web_atoms::{Atom, Namespace, QualName};

use crate::sink::{Attr, Attributes, TreeSink};
use crate::tokenizer::ENTITIES;

const MAX_DEPTH: usize = 512;
const DECLARATION_SCAN: usize = 1024;

const XHTML_URI: &str = "http://www.w3.org/1999/xhtml";
const SVG_URI: &str = "http://www.w3.org/2000/svg";
const MATHML_URI: &str = "http://www.w3.org/1998/Math/MathML";
const XLINK_URI: &str = "http://www.w3.org/1999/xlink";
const XML_URI: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_URI: &str = "http://www.w3.org/2000/xmlns/";

const NAMED_ENTITY_DOCTYPES: [&str; 9] = [
    "-//W3C//DTD XHTML 1.0 Transitional//EN",
    "-//W3C//DTD XHTML 1.1//EN",
    "-//W3C//DTD XHTML 1.0 Strict//EN",
    "-//W3C//DTD XHTML 1.0 Frameset//EN",
    "-//W3C//DTD XHTML Basic 1.0//EN",
    "-//W3C//DTD XHTML 1.1 plus MathML 2.0//EN",
    "-//W3C//DTD XHTML 1.1 plus MathML 2.0 plus SVG 1.1//EN",
    "-//W3C//DTD MathML 2.0//EN",
    "-//WAPFORUM//DTD XHTML Mobile 1.0//EN",
];

pub fn parse_xml<S: TreeSink>(sink: S, bytes: &[u8], transport: Option<&'static Encoding>) -> S {
    let text = decode(bytes, transport);
    let mut parser = XmlParser::new(sink, text.as_deref().unwrap_or_default());
    let outcome = match text {
        Some(_) => parser.run(),
        None => Err(XmlError {
            offset: 0,
            message: "the bytes are not valid in the document's encoding",
        }),
    };
    if let Err(error) = outcome {
        parser.fail(error);
    }
    parser.sink
}

fn decode(bytes: &[u8], transport: Option<&'static Encoding>) -> Option<String> {
    let (encoding, bom_length) = Encoding::for_bom(bytes).unwrap_or_else(|| {
        let declared = declared_encoding(bytes).filter(|&e| e != UTF_16BE && e != UTF_16LE);
        (transport.or(declared).unwrap_or(UTF_8), 0)
    });
    let bytes = bytes.get(bom_length..).unwrap_or_default();
    let text = encoding.decode_without_bom_handling_and_without_replacement(bytes)?;
    if !text.contains('\r') {
        return Some(text.into_owned());
    }
    Some(text.replace("\r\n", "\n").replace('\r', "\n"))
}

fn declared_encoding(bytes: &[u8]) -> Option<&'static Encoding> {
    let head = bytes.get(..DECLARATION_SCAN.min(bytes.len()))?;
    let declaration = head.strip_prefix(b"<?xml")?;
    let end = declaration.windows(2).position(|pair| pair == b"?>")?;
    let declaration = declaration.get(..end)?;
    let at = declaration
        .windows(8)
        .position(|window| window == b"encoding")?;
    let rest = declaration.get(at + 8..)?.trim_ascii_start();
    let rest = rest.strip_prefix(b"=")?.trim_ascii_start();
    let (&quote, rest) = rest.split_first()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let label = rest.get(..rest.iter().position(|&b| b == quote)?)?;
    Encoding::for_label(label)
}

struct XmlError {
    offset: usize,
    message: &'static str,
}

struct Open<'a, N> {
    parent: N,
    name: &'a str,
    mark: usize,
}

#[derive(Default)]
struct Scopes<'a> {
    bound: HashMap<&'a str, Vec<(Namespace, String)>>,
    undo: Vec<&'a str>,
}

impl<'a> Scopes<'a> {
    fn lookup(&self, prefix: &str) -> Option<(Namespace, &str)> {
        if prefix == "xml" {
            return Some((Namespace::Xml, XML_URI));
        }
        let (namespace, uri) = self.bound.get(prefix)?.last()?;
        Some((*namespace, uri.as_str()))
    }

    fn resolve(&self, qualified: &'a str, element: bool) -> Option<(Namespace, &str, &'a str)> {
        if !element && qualified == "xmlns" {
            return Some((Namespace::Xmlns, XMLNS_URI, qualified));
        }
        let Some((prefix, local)) = qualified.split_once(':') else {
            let (ns, uri) = match element {
                true => self.lookup("").unwrap_or((Namespace::None, "")),
                false => (Namespace::None, ""),
            };
            return Some((ns, uri, qualified));
        };
        if prefix.is_empty() || local.contains(':') || !local.starts_with(is_name_start_char) {
            return None;
        }
        if prefix == "xmlns" {
            return (!element).then_some((Namespace::Xmlns, XMLNS_URI, local));
        }
        let (ns, uri) = self.lookup(prefix)?;
        Some((ns, uri, local))
    }

    fn bind(&mut self, prefix: &'a str, uri: &str) {
        self.bound
            .entry(prefix)
            .or_default()
            .push((namespace(uri), uri.to_owned()));
        self.undo.push(prefix);
    }

    fn unbind_to(&mut self, mark: usize) {
        while self.undo.len() > mark {
            if let Some(prefix) = self.undo.pop()
                && let Some(stack) = self.bound.get_mut(prefix)
            {
                stack.pop();
            }
        }
    }
}

fn namespace(uri: &str) -> Namespace {
    match uri {
        "" => Namespace::None,
        XHTML_URI => Namespace::Html,
        SVG_URI => Namespace::Svg,
        MATHML_URI => Namespace::MathMl,
        XLINK_URI => Namespace::XLink,
        XML_URI => Namespace::Xml,
        XMLNS_URI => Namespace::Xmlns,
        _ => Namespace::Other,
    }
}

struct XmlParser<'a, S: TreeSink> {
    sink: S,
    text: &'a str,
    pos: usize,
    open: Vec<Open<'a, S::Node>>,
    scopes: Scopes<'a>,
    top_level: Vec<S::Node>,
    pending: String,
    values: String,
    raw: Vec<(&'a str, usize, usize)>,
    attrs: Vec<Attr>,
    root_seen: bool,
    doctype_seen: bool,
    named_entities: bool,
    external_markup: bool,
}

impl<'a, S: TreeSink> XmlParser<'a, S> {
    fn new(sink: S, text: &'a str) -> Self {
        XmlParser {
            sink,
            text,
            pos: 0,
            open: Vec::new(),
            scopes: Scopes::default(),
            top_level: Vec::new(),
            pending: String::new(),
            values: String::new(),
            raw: Vec::new(),
            attrs: Vec::new(),
            root_seen: false,
            doctype_seen: false,
            named_entities: false,
            external_markup: false,
        }
    }

    fn run(&mut self) -> Result<(), XmlError> {
        self.xml_declaration()?;
        while !self.rest().is_empty() {
            if self.rest().starts_with('<') {
                self.markup()?;
            } else {
                self.characters()?;
            }
        }
        if !self.open.is_empty() {
            return self.error("the document ends inside an element");
        }
        if !self.root_seen {
            return self.error("the document has no root element");
        }
        Ok(())
    }

    fn fail(&mut self, error: XmlError) {
        for node in self.top_level.drain(..) {
            self.sink.detach(node);
        }
        let before = self.text.get(..error.offset).unwrap_or_default();
        let line = before.matches('\n').count() + 1;
        let column = before
            .rsplit('\n')
            .next()
            .unwrap_or_default()
            .chars()
            .count()
            + 1;
        let message = format!(
            "XML parse error: {} at line {line}, column {column}",
            error.message
        );
        let local = self.sink.intern("parsererror");
        let element = self.sink.create_element(
            QualName::html(local),
            Attributes {
                list: &[],
                values: "",
            },
        );
        let document = self.sink.document();
        self.sink.insert(document, element, None);
        self.sink.insert_text(element, None, &message);
    }

    fn rest(&self) -> &'a str {
        self.text.get(self.pos..).unwrap_or_default()
    }

    fn error<T>(&self, message: &'static str) -> Result<T, XmlError> {
        Err(XmlError {
            offset: self.pos,
            message,
        })
    }

    fn eat(&mut self, c: char) -> bool {
        let found = self.rest().starts_with(c);
        if found {
            self.pos += c.len_utf8();
        }
        found
    }

    fn skip_space(&mut self) -> bool {
        let rest = self.rest();
        let skipped = rest.len() - rest.trim_start_matches(is_space).len();
        self.pos += skipped;
        skipped > 0
    }

    fn name(&mut self) -> Result<&'a str, XmlError> {
        let rest = self.rest();
        let length = name_length(rest);
        if length == 0 {
            return self.error("expected a name");
        }
        self.pos += length;
        Ok(rest.get(..length).unwrap_or_default())
    }

    fn check_chars(&self, run: &str) -> Result<(), XmlError> {
        match run.char_indices().find(|&(_, c)| !is_xml_char(c)) {
            Some((at, _)) => Err(XmlError {
                offset: self.pos + at,
                message: "a character that XML does not allow",
            }),
            None => Ok(()),
        }
    }

    fn until(&mut self, skip: usize, close: &str) -> Result<&'a str, XmlError> {
        self.pos += skip;
        let rest = self.rest();
        let Some(end) = rest.find(close) else {
            self.pos = self.text.len();
            return self.error("unterminated markup");
        };
        let body = rest.get(..end).unwrap_or_default();
        self.check_chars(body)?;
        self.pos += end + close.len();
        Ok(body)
    }

    fn xml_declaration(&mut self) -> Result<(), XmlError> {
        let rest = self.rest();
        let Some(after) = rest.strip_prefix("<?xml") else {
            return Ok(());
        };
        if !after.starts_with(is_space) {
            return Ok(());
        }
        let body = self.until(5, "?>")?;
        let mut names = ["version", "encoding", "standalone"].as_slice();
        let mut rest = body;
        loop {
            let trimmed = rest.trim_start_matches(is_space);
            if trimmed.is_empty() {
                break;
            }
            let pair = (trimmed.len() < rest.len())
                .then(|| trimmed.split_once('='))
                .flatten()
                .and_then(|(name, value)| {
                    let value = value.trim_start_matches(is_space);
                    let quote = value.chars().next().filter(|&q| q == '"' || q == '\'')?;
                    let (value, after) = value.get(1..)?.split_once(quote)?;
                    Some((name.trim_end_matches(is_space), value, after))
                });
            let Some((name, value, after)) = pair else {
                return self.error("malformed XML declaration");
            };
            let Some(index) = names.iter().position(|&n| n == name) else {
                return self.error("malformed XML declaration");
            };
            let version = name == "version";
            if (names.len() == 3) != version || (version && !value.starts_with("1.")) {
                return self.error("malformed XML declaration");
            }
            names = names.get(index + 1..).unwrap_or_default();
            rest = after;
        }
        if names.len() == 3 {
            return self.error("the XML declaration has no version");
        }
        Ok(())
    }

    fn characters(&mut self) -> Result<(), XmlError> {
        let rest = self.rest();
        let end = rest.find(['<', '&']).unwrap_or(rest.len());
        let run = rest.get(..end).unwrap_or_default();
        if self.open.is_empty() {
            if let Some(at) = run.find(|c| !is_space(c)) {
                self.pos += at;
                return self.error("text outside the root element");
            }
            self.pos += end;
            if self.rest().starts_with('&') {
                return self.error("a reference outside the root element");
            }
            return Ok(());
        }
        if let Some(at) = run.find("]]>") {
            self.pos += at;
            return self.error("]]> in text");
        }
        self.check_chars(run)?;
        self.pending.push_str(run);
        self.pos += end;
        if self.rest().starts_with('&') {
            let mut pending = std::mem::take(&mut self.pending);
            let result = self.reference(&mut pending);
            self.pending = pending;
            result?;
        }
        Ok(())
    }

    fn reference(&mut self, out: &mut String) -> Result<(), XmlError> {
        let rest = self.rest().get(1..).unwrap_or_default();
        if let Some(number) = rest.strip_prefix('#') {
            let (digits, radix, prefix) = match number.strip_prefix('x') {
                Some(hex) => (hex, 16, 3),
                None => (number, 10, 2),
            };
            let length = digits
                .find(|c: char| !c.is_digit(radix))
                .unwrap_or(digits.len());
            let value = digits
                .get(..length)
                .filter(|digits| !digits.is_empty())
                .filter(|_| {
                    digits
                        .get(length..)
                        .is_some_and(|after| after.starts_with(';'))
                })
                .and_then(|digits| u32::from_str_radix(digits, radix).ok())
                .and_then(char::from_u32)
                .filter(|&c| is_xml_char(c));
            let Some(c) = value else {
                return self.error("a malformed or invalid character reference");
            };
            out.push(c);
            self.pos += prefix + length + 1;
            return Ok(());
        }
        let length = name_length(rest);
        let name = rest.get(..length).unwrap_or_default();
        if length == 0
            || !rest
                .get(length..)
                .is_some_and(|after| after.starts_with(';'))
        {
            return self.error("a malformed entity reference");
        }
        let predefined = match name {
            "lt" => Some(['<', '\0']),
            "gt" => Some(['>', '\0']),
            "amp" => Some(['&', '\0']),
            "apos" => Some(['\'', '\0']),
            "quot" => Some(['"', '\0']),
            _ => None,
        };
        let expansion = predefined.or_else(|| {
            self.named_entities
                .then(|| {
                    let key = name.bytes().chain(std::iter::once(b';'));
                    ENTITIES
                        .binary_search_by(|entry| entry.0.bytes().cmp(key.clone()))
                        .ok()
                })
                .flatten()
                .and_then(|index| ENTITIES.get(index))
                .map(|&(_, first, second)| [first, second])
        });
        match expansion {
            Some(chars) => out.extend(chars.into_iter().filter(|&c| c != '\0')),
            None if self.external_markup => {}
            None => return self.error("an undefined entity"),
        }
        self.pos += 1 + length + 1;
        Ok(())
    }

    fn markup(&mut self) -> Result<(), XmlError> {
        let rest = self.rest();
        if rest.starts_with("<?") {
            self.processing_instruction()
        } else if rest.starts_with("<!--") {
            self.comment()
        } else if rest.starts_with("<![CDATA[") {
            self.cdata()
        } else if rest.starts_with("<!DOCTYPE") {
            self.doctype()
        } else if rest.starts_with("</") {
            self.end_tag()
        } else {
            self.start_tag()
        }
    }

    fn insert(&mut self, node: S::Node) {
        match self.open.last() {
            Some(open) => {
                let parent = open.parent;
                self.flush(parent);
                self.sink.insert(parent, node, None);
            }
            None => {
                let document = self.sink.document();
                self.sink.insert(document, node, None);
                self.top_level.push(node);
            }
        }
    }

    fn flush(&mut self, parent: S::Node) {
        if !self.pending.is_empty() {
            self.sink.insert_text(parent, None, &self.pending);
            self.pending.clear();
        }
    }

    fn processing_instruction(&mut self) -> Result<(), XmlError> {
        self.pos += 2;
        let target = self.name()?;
        if target.eq_ignore_ascii_case("xml") {
            return self.error("an XML declaration that is not at the start");
        }
        if target.contains(':') {
            return self.error("a processing instruction target with a colon");
        }
        let data = if self.rest().starts_with("?>") {
            self.pos += 2;
            ""
        } else if self.skip_space() {
            self.until(0, "?>")?
        } else {
            return self.error("expected whitespace after the processing instruction target");
        };
        let node = self.sink.create_processing_instruction(target, data);
        self.insert(node);
        Ok(())
    }

    fn comment(&mut self) -> Result<(), XmlError> {
        self.pos += 4;
        let rest = self.rest();
        let Some(dashes) = rest.find("--") else {
            self.pos = self.text.len();
            return self.error("unterminated comment");
        };
        let body = rest.get(..dashes).unwrap_or_default();
        self.check_chars(body)?;
        self.pos += dashes;
        if !self.rest().starts_with("-->") {
            return self.error("-- inside a comment");
        }
        self.pos += 3;
        let node = self.sink.create_comment(body);
        self.insert(node);
        Ok(())
    }

    fn cdata(&mut self) -> Result<(), XmlError> {
        if self.open.is_empty() {
            return self.error("a CDATA section outside the root element");
        }
        let body = self.until(9, "]]>")?;
        self.pending.push_str(body);
        Ok(())
    }

    fn doctype(&mut self) -> Result<(), XmlError> {
        if self.doctype_seen || self.root_seen {
            return self.error("a misplaced DOCTYPE");
        }
        self.doctype_seen = true;
        self.pos += 9;
        if !self.skip_space() {
            return self.error("expected whitespace after DOCTYPE");
        }
        let name = self.name()?;
        let spaced = self.skip_space();
        let rest = self.rest();
        let (mut public, mut system) = ("", "");
        if spaced && (rest.starts_with("PUBLIC") || rest.starts_with("SYSTEM")) {
            self.pos += 6;
            if !self.skip_space() {
                return self.error("expected whitespace before the identifier");
            }
            if rest.starts_with("PUBLIC") {
                public = self.literal()?;
                if !self.skip_space() {
                    return self.error("expected whitespace before the system identifier");
                }
            }
            system = self.literal()?;
            self.external_markup = true;
            self.skip_space();
        }
        if self.eat('[') {
            self.internal_subset()?;
            self.external_markup = true;
            self.skip_space();
        }
        if !self.eat('>') {
            return self.error("a malformed DOCTYPE");
        }
        self.named_entities = NAMED_ENTITY_DOCTYPES.contains(&public);
        self.sink.append_doctype(name, public, system);
        Ok(())
    }

    fn literal(&mut self) -> Result<&'a str, XmlError> {
        match self.rest().chars().next() {
            Some('"') => self.until(1, "\""),
            Some('\'') => self.until(1, "'"),
            _ => self.error("expected a quoted identifier"),
        }
    }

    fn internal_subset(&mut self) -> Result<(), XmlError> {
        loop {
            let rest = self.rest();
            let Some(at) = rest.find(['"', '\'', '<', ']']) else {
                self.pos = self.text.len();
                return self.error("unterminated internal subset");
            };
            self.pos += at;
            let rest = self.rest();
            if rest.starts_with(']') {
                self.pos += 1;
                return Ok(());
            }
            if rest.starts_with("<!--") {
                self.until(4, "-->")?;
            } else if rest.starts_with("<?") {
                self.until(2, "?>")?;
            } else if rest.starts_with('"') {
                self.until(1, "\"")?;
            } else if rest.starts_with('\'') {
                self.until(1, "'")?;
            } else {
                self.pos += 1;
            }
        }
    }

    fn start_tag(&mut self) -> Result<(), XmlError> {
        if self.open.is_empty() && self.root_seen {
            return self.error("markup after the root element");
        }
        if self.open.len() >= MAX_DEPTH {
            return self.error("elements nested too deeply");
        }
        self.pos += 1;
        let name = self.name()?;
        self.raw.clear();
        self.values.clear();
        let self_closing = loop {
            let spaced = self.skip_space();
            let rest = self.rest();
            if rest.starts_with("/>") {
                self.pos += 2;
                break true;
            }
            if self.eat('>') {
                break false;
            }
            if !spaced {
                return self.error("expected whitespace, > or /> in a start tag");
            }
            let attribute = self.name()?;
            self.skip_space();
            if !self.eat('=') {
                return self.error("expected = after an attribute name");
            }
            self.skip_space();
            let start = self.values.len();
            self.attribute_value()?;
            self.raw.push((attribute, start, self.values.len()));
        };
        self.open_element(name, self_closing)
    }

    fn attribute_value(&mut self) -> Result<(), XmlError> {
        let quote = match self.rest().chars().next() {
            Some(quote @ ('"' | '\'')) => quote,
            _ => return self.error("attribute values must be quoted"),
        };
        self.pos += 1;
        let mut values = std::mem::take(&mut self.values);
        let result = self.attribute_text(quote, &mut values);
        self.values = values;
        result
    }

    fn attribute_text(&mut self, quote: char, values: &mut String) -> Result<(), XmlError> {
        loop {
            let rest = self.rest();
            let Some(end) = rest.find([quote, '<', '&']) else {
                self.pos = self.text.len();
                return self.error("unterminated attribute value");
            };
            let run = rest.get(..end).unwrap_or_default();
            self.check_chars(run)?;
            values.extend(
                run.chars()
                    .map(|c| if matches!(c, '\t' | '\n') { ' ' } else { c }),
            );
            self.pos += end;
            if self.rest().starts_with('&') {
                self.reference(values)?;
            } else if self.rest().starts_with('<') {
                return self.error("< in an attribute value");
            } else {
                self.pos += 1;
                return Ok(());
            }
        }
    }

    fn open_element(&mut self, name: &'a str, self_closing: bool) -> Result<(), XmlError> {
        let raw = std::mem::take(&mut self.raw);
        let result = self.create_element(name, &raw);
        self.raw = raw;
        let (element, name_of_element, mark) = result?;
        let parent = match name_of_element == QualName::html(Atom::TEMPLATE) {
            true => self.sink.template_contents(element),
            false => element,
        };
        self.insert(element);
        self.root_seen = true;
        if self_closing {
            self.scopes.unbind_to(mark);
        } else {
            self.open.push(Open { parent, name, mark });
        }
        Ok(())
    }

    fn create_element(
        &mut self,
        name: &'a str,
        raw: &[(&'a str, usize, usize)],
    ) -> Result<(S::Node, QualName, usize), XmlError> {
        let mark = self.scopes.undo.len();
        let start = self.pos;
        for &(qualified, from, to) in raw {
            let prefix = match qualified.strip_prefix("xmlns") {
                Some("") => "",
                Some(rest) => match rest.strip_prefix(':') {
                    Some(prefix) => prefix,
                    None => continue,
                },
                None => continue,
            };
            let uri = self.values.get(from..to).unwrap_or_default();
            let allowed = match prefix {
                "xmlns" => false,
                "xml" => uri == XML_URI,
                _ => uri != XML_URI && uri != XMLNS_URI && (prefix.is_empty() || !uri.is_empty()),
            };
            if !allowed {
                self.scopes.unbind_to(mark);
                return self.error("a reserved or empty namespace declaration");
            }
            if prefix != "xml" {
                self.scopes.bind(prefix, uri);
            }
        }
        let scopes = &self.scopes;
        let sink = &mut self.sink;
        let attrs = &mut self.attrs;
        let resolved = scopes.resolve(name, true).and_then(|(ns, _, local)| {
            attrs.clear();
            let mut seen = HashSet::new();
            for &(qualified, from, to) in raw {
                let (ns, uri, local) = scopes.resolve(qualified, false)?;
                if !seen.insert((uri, local)) {
                    return None;
                }
                attrs.push(Attr {
                    name: QualName {
                        ns,
                        local: sink.intern(local),
                    },
                    start: u32::try_from(from).ok()?,
                    end: u32::try_from(to).ok()?,
                });
            }
            Some((ns, local))
        });
        let Some((ns, local)) = resolved else {
            self.scopes.unbind_to(mark);
            self.pos = start;
            return self.error("an unbound prefix, malformed name or duplicate attribute");
        };
        let name = QualName {
            ns,
            local: self.sink.intern(local),
        };
        let element = self.sink.create_element(
            name,
            Attributes {
                list: &self.attrs,
                values: &self.values,
            },
        );
        Ok((element, name, mark))
    }

    fn end_tag(&mut self) -> Result<(), XmlError> {
        let start = self.pos;
        self.pos += 2;
        let name = self.name()?;
        self.skip_space();
        if !self.eat('>') {
            return self.error("expected > at the end of an end tag");
        }
        let Some(open) = self.open.pop() else {
            self.pos = start;
            return self.error("an end tag without a start tag");
        };
        if open.name != name {
            self.pos = start;
            return self.error("an end tag that does not match the open element");
        }
        self.flush(open.parent);
        self.scopes.unbind_to(open.mark);
        Ok(())
    }
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | ' '..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

fn is_name_start_char(c: char) -> bool {
    matches!(c,
        ':' | 'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

fn is_name_char(c: char) -> bool {
    is_name_start_char(c)
        || matches!(c, '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

fn name_length(text: &str) -> usize {
    let mut chars = text.char_indices();
    if !chars.next().is_some_and(|(_, c)| is_name_start_char(c)) {
        return 0;
    }
    chars
        .find(|&(_, c)| !is_name_char(c))
        .map_or(text.len(), |(at, _)| at)
}
