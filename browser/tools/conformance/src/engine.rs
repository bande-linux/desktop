use std::borrow::Cow;
use std::path::Path;

use serde_json::{Map, Value, json};
use web_content::Pixels;
use web_css::{
    BlockContentsParser, ParseError, ParseResult, Parser, RuleHandler, RuleListParser, Token,
    Tokenizer, preprocess,
};
use web_html::{
    Atom, AtomTable, Attributes, Namespace, Options, QualName, QuirksMode, TokenizerState, TreeSink,
};

use crate::reftest::{HEIGHT, WIDTH};
use crate::tree::{self, Context, Node};

pub fn tokenize(input: &str, state: &str, last_start_tag: Option<&str>) -> Vec<Value> {
    let state = match state {
        "Data state" => TokenizerState::Data,
        "PLAINTEXT state" => TokenizerState::Plaintext,
        "RCDATA state" => TokenizerState::Rcdata,
        "RAWTEXT state" => TokenizerState::Rawtext,
        "Script data state" => TokenizerState::ScriptData,
        "CDATA section state" => TokenizerState::CdataSection,
        _ => return Vec::new(),
    };
    let mut tokens = Vec::new();
    let mut atoms = AtomTable::new();
    web_html::tokenize(input, state, last_start_tag, &mut atoms, |token, atoms| {
        tokens.push(match token {
            web_html::Token::Characters(text) => json!(["Character", text]),
            web_html::Token::StartTag {
                name,
                attributes,
                self_closing,
            } => {
                let mut map = Map::new();
                for (name, value) in attributes.iter() {
                    map.entry(atoms.name(name.local))
                        .or_insert_with(|| json!(value));
                }
                let mut tag = vec![
                    json!("StartTag"),
                    json!(atoms.name(name)),
                    Value::Object(map),
                ];
                if self_closing {
                    tag.push(json!(true));
                }
                Value::Array(tag)
            }
            web_html::Token::EndTag { name } => json!(["EndTag", atoms.name(name)]),
            web_html::Token::Comment(text) => json!(["Comment", text]),
            web_html::Token::ProcessingInstruction { target, data } => {
                json!(["ProcessingInstruction", target, data])
            }
            web_html::Token::Doctype {
                name,
                public_id,
                system_id,
                force_quirks,
            } => json!(["DOCTYPE", name, public_id, system_id, !force_quirks]),
        });
    });
    tokens
}

pub fn parse(input: &str, context: Option<&Context>, scripting: bool) -> Vec<Node> {
    let options = Options {
        scripting,
        ..Options::default()
    };
    let mut sink = Sink::new();
    let Some(context) = context else {
        let mut parser = web_html::Parser::new(sink, options);
        parser.feed_str(input);
        return parser.finish().export(0);
    };
    let local = sink.atoms.intern(context.name);
    let ns = match context.namespace {
        tree::Namespace::Html => Namespace::Html,
        tree::Namespace::Svg => Namespace::Svg,
        tree::Namespace::MathMl => Namespace::MathMl,
    };
    let mut parser = web_html::Parser::new_fragment(sink, QualName { ns, local }, options);
    parser.feed_str(input);
    let sink = parser.finish();
    let root = sink
        .nodes
        .first()
        .and_then(|document| document.children.first().copied());
    root.map(|root| sink.export(root)).unwrap_or_default()
}

pub fn render(path: &Path) -> Option<Pixels> {
    web_content::render_file(path, WIDTH, HEIGHT).ok()
}

pub fn css(entry: &str, input: &Value) -> Option<Value> {
    let css = preprocess(input.as_str()?);
    let mut tokenizer = if entry == "component_value_list" {
        Tokenizer::with_unicode_ranges(&css)
    } else {
        Tokenizer::new(&css)
    };
    let mut parser = Parser::new(&mut tokenizer);
    let input = &mut parser;
    let nested = |nested_rules| Collect { nested_rules };
    Some(match entry {
        "component_value_list" => Value::Array(component_values(input)),
        "one_component_value" => one_component_value(input),
        "declaration_list" => results(BlockContentsParser::new(input, &mut nested(false))),
        "blocks_contents" => results(BlockContentsParser::new(input, &mut nested(true))),
        "one_declaration" => one_declaration(input),
        "one_rule" => one_rule(input),
        "rule_list" => results(RuleListParser::new_for_nested_rules(
            input,
            &mut nested(true),
        )),
        "stylesheet" => results(RuleListParser::new_for_stylesheet(input, &mut nested(true))),
        "An+B" => an_plus_b(input),
        _ => return None,
    })
}

struct Sink {
    atoms: AtomTable,
    nodes: Vec<Entry>,
}

struct Entry {
    kind: Kind,
    parent: Option<usize>,
    children: Vec<usize>,
}

enum Kind {
    Root,
    Doctype(String, String, String),
    Element(QualName, Vec<(QualName, String)>, Option<usize>),
    Text(String),
    Comment(String),
    ProcessingInstruction(String, String),
}

impl Sink {
    fn new() -> Self {
        Self {
            atoms: AtomTable::new(),
            nodes: vec![Entry {
                kind: Kind::Root,
                parent: None,
                children: Vec::new(),
            }],
        }
    }

    fn push(&mut self, kind: Kind) -> usize {
        self.nodes.push(Entry {
            kind,
            parent: None,
            children: Vec::new(),
        });
        self.nodes.len() - 1
    }

    fn position(&self, parent: usize, before: Option<usize>) -> usize {
        let children = self
            .nodes
            .get(parent)
            .map_or(&[][..], |entry| &entry.children);
        before
            .and_then(|before| children.iter().position(|&child| child == before))
            .unwrap_or(children.len())
    }

    fn attach(&mut self, parent: usize, child: usize, index: usize) {
        if let Some(entry) = self.nodes.get_mut(parent) {
            entry
                .children
                .insert(index.min(entry.children.len()), child);
        }
        if let Some(entry) = self.nodes.get_mut(child) {
            entry.parent = Some(parent);
        }
    }

    fn collect_attributes(&self, attributes: Attributes<'_>) -> Vec<(QualName, String)> {
        attributes
            .iter()
            .map(|(name, value)| (name, value.to_owned()))
            .collect()
    }

    fn export(&self, node: usize) -> Vec<Node> {
        let children = self
            .nodes
            .get(node)
            .map_or(&[][..], |entry| &entry.children);
        children
            .iter()
            .filter_map(|&child| self.export_one(child))
            .collect()
    }

    fn export_one(&self, node: usize) -> Option<Node> {
        Some(match &self.nodes.get(node)?.kind {
            Kind::Root => return None,
            Kind::Doctype(name, public_id, system_id) => Node::Doctype {
                name: name.clone(),
                public_id: (!public_id.is_empty() || !system_id.is_empty())
                    .then(|| public_id.clone()),
                system_id: (!public_id.is_empty() || !system_id.is_empty())
                    .then(|| system_id.clone()),
            },
            Kind::Element(name, attributes, contents) => Node::Element {
                namespace: match name.ns {
                    Namespace::Svg => tree::Namespace::Svg,
                    Namespace::MathMl => tree::Namespace::MathMl,
                    _ => tree::Namespace::Html,
                },
                name: self.atoms.name(name.local).to_owned(),
                attributes: attributes
                    .iter()
                    .map(|(name, value)| {
                        let local = self.atoms.name(name.local);
                        let key = match name.ns {
                            Namespace::XLink => format!("xlink {local}"),
                            Namespace::Xml => format!("xml {local}"),
                            Namespace::Xmlns => format!("xmlns {local}"),
                            _ => local.to_owned(),
                        };
                        (key, value.clone())
                    })
                    .collect(),
                children: self.export(node),
                template_contents: match contents {
                    Some(contents) => Some(self.export(*contents)),
                    None => (name.ns == Namespace::Html
                        && self.atoms.name(name.local) == "template")
                        .then(Vec::new),
                },
            },
            Kind::Text(text) => Node::Text(text.clone()),
            Kind::Comment(text) => Node::Comment(text.clone()),
            Kind::ProcessingInstruction(target, data) => Node::ProcessingInstruction {
                target: target.clone(),
                data: data.clone(),
            },
        })
    }
}

impl TreeSink for Sink {
    type Node = usize;

    fn intern(&mut self, name: &str) -> Atom {
        self.atoms.intern(name)
    }

    fn document(&self) -> usize {
        0
    }

    fn create_element(&mut self, name: QualName, attributes: Attributes<'_>) -> usize {
        let attributes = self.collect_attributes(attributes);
        self.push(Kind::Element(name, attributes, None))
    }

    fn create_comment(&mut self, text: &str) -> usize {
        self.push(Kind::Comment(text.to_owned()))
    }

    fn create_processing_instruction(&mut self, target: &str, data: &str) -> usize {
        self.push(Kind::ProcessingInstruction(
            target.to_owned(),
            data.to_owned(),
        ))
    }

    fn append_doctype(&mut self, name: &str, public_id: &str, system_id: &str) {
        let doctype = self.push(Kind::Doctype(
            name.to_owned(),
            public_id.to_owned(),
            system_id.to_owned(),
        ));
        let index = self.position(0, None);
        self.attach(0, doctype, index);
    }

    fn insert(&mut self, parent: usize, child: usize, before: Option<usize>) {
        self.detach(child);
        let index = self.position(parent, before);
        self.attach(parent, child, index);
    }

    fn insert_text(&mut self, parent: usize, before: Option<usize>, text: &str) {
        let index = self.position(parent, before);
        let previous = index
            .checked_sub(1)
            .and_then(|previous| self.nodes.get(parent)?.children.get(previous).copied());
        if let Some(Entry {
            kind: Kind::Text(existing),
            ..
        }) = previous.and_then(|previous| self.nodes.get_mut(previous))
        {
            existing.push_str(text);
            return;
        }
        let node = self.push(Kind::Text(text.to_owned()));
        self.attach(parent, node, index);
    }

    fn parent(&self, node: usize) -> Option<usize> {
        self.nodes.get(node)?.parent
    }

    fn detach(&mut self, node: usize) {
        let Some(parent) = self
            .nodes
            .get_mut(node)
            .and_then(|entry| entry.parent.take())
        else {
            return;
        };
        if let Some(entry) = self.nodes.get_mut(parent) {
            entry.children.retain(|&child| child != node);
        }
    }

    fn reparent_children(&mut self, from: usize, to: usize) {
        let children = self
            .nodes
            .get_mut(from)
            .map(|entry| std::mem::take(&mut entry.children))
            .unwrap_or_default();
        for child in children {
            if let Some(entry) = self.nodes.get_mut(child) {
                entry.parent = None;
            }
            let index = self.position(to, None);
            self.attach(to, child, index);
        }
    }

    fn add_attributes_if_missing(&mut self, element: usize, attributes: Attributes<'_>) {
        let added = self.collect_attributes(attributes);
        if let Some(Entry {
            kind: Kind::Element(_, existing, _),
            ..
        }) = self.nodes.get_mut(element)
        {
            for (name, value) in added {
                if !existing.iter().any(|(present, _)| *present == name) {
                    existing.push((name, value));
                }
            }
        }
    }

    fn template_contents(&mut self, template: usize) -> usize {
        if let Some(Entry {
            kind: Kind::Element(_, _, Some(contents)),
            ..
        }) = self.nodes.get(template)
        {
            return *contents;
        }
        let contents = self.push(Kind::Root);
        if let Some(Entry {
            kind: Kind::Element(_, _, slot),
            ..
        }) = self.nodes.get_mut(template)
        {
            *slot = Some(contents);
        }
        contents
    }

    fn set_quirks_mode(&mut self, _mode: QuirksMode) {}
}

fn an_plus_b(input: &mut Parser<'_, '_>) -> Value {
    input.skip_whitespace();
    let parsed = web_css::nth::parse_nth(input);
    input.skip_whitespace();
    match parsed {
        Ok((a, b)) if input.is_exhausted() => json!([a, b]),
        _ => Value::Null,
    }
}

fn error(kind: &str) -> Value {
    json!(["error", kind])
}

fn strip_comments(mut text: &str) -> &str {
    while let Some(rest) = text.strip_prefix("/*") {
        text = rest.find("*/").map_or("", |end| &rest[end + 2..]);
    }
    text
}

fn number_repr(text: &str) -> &str {
    let bytes = text.as_bytes();
    let digit = |index: usize| bytes.get(index).is_some_and(u8::is_ascii_digit);
    let mut end = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    while digit(end) {
        end += 1;
    }
    if bytes.get(end) == Some(&b'.') && digit(end + 1) {
        end += 1;
        while digit(end) {
            end += 1;
        }
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(bytes.get(end + 1), Some(b'+' | b'-')));
        if digit(end + 1 + sign) {
            end += 1 + sign;
            while digit(end) {
                end += 1;
            }
        }
    }
    text.get(..end).unwrap_or(text)
}

fn ends_unescaped(text: &str, end: char) -> bool {
    text.strip_suffix(end)
        .is_some_and(|body| body.chars().rev().take_while(|&c| c == '\\').count() % 2 == 0)
}

fn numeric(kind: &str, repr: &str, number: &web_css::Numeric) -> Vec<Value> {
    let kind_name = if number.is_integer {
        "integer"
    } else {
        "number"
    };
    vec![
        json!(kind),
        json!(repr),
        json!(number.value),
        json!(kind_name),
    ]
}

fn component_value(input: &mut Parser<'_, '_>, token: Token<'_>, source: &str) -> Vec<Value> {
    let value = match token {
        Token::Ident(name) => json!(["ident", name]),
        Token::AtKeyword(name) => json!(["at-keyword", name]),
        Token::Hash(name) => json!(["hash", name, "unrestricted"]),
        Token::IdHash(name) => json!(["hash", name, "id"]),
        Token::String(text) => {
            let quote = source.chars().next().unwrap_or('"');
            let mut values = vec![json!(["string", text])];
            if source.len() < 2 || !ends_unescaped(source, quote) {
                values.push(error("eof-in-string"));
            }
            return values;
        }
        Token::BadString => error("bad-string"),
        Token::Url(url) => {
            let mut values = vec![json!(["url", url])];
            if !ends_unescaped(source, ')') {
                values.push(error("eof-in-url"));
            }
            return values;
        }
        Token::BadUrl => error("bad-url"),
        Token::Delim(c) => json!(c.to_string()),
        Token::Number(n) => Value::Array(numeric("number", number_repr(source), &n)),
        Token::Percentage(n) => Value::Array(numeric("percentage", number_repr(source), &n)),
        Token::Dimension(n, unit) => {
            let mut items = numeric("dimension", number_repr(source), &n);
            items.push(json!(unit));
            Value::Array(items)
        }
        Token::UnicodeRange(start, end) => json!(["unicode-range", start, end]),
        Token::Whitespace => json!(" "),
        Token::Cdo => json!("<!--"),
        Token::Cdc => json!("-->"),
        Token::Colon => json!(":"),
        Token::Semicolon => json!(";"),
        Token::Comma => json!(","),
        Token::IncludeMatch => json!("~="),
        Token::DashMatch => json!("|="),
        Token::PrefixMatch => json!("^="),
        Token::SuffixMatch => json!("$="),
        Token::SubstringMatch => json!("*="),
        Token::Column => json!("||"),
        Token::CloseSquare => error("]"),
        Token::CloseParen => error(")"),
        Token::CloseCurly => error("}"),
        Token::Function(name) => {
            let mut items = vec![json!("function"), json!(name)];
            items.extend(nested(input));
            Value::Array(items)
        }
        Token::OpenParen => block("()", input),
        Token::OpenSquare => block("[]", input),
        Token::OpenCurly => block("{}", input),
    };
    vec![value]
}

fn block(kind: &str, input: &mut Parser<'_, '_>) -> Value {
    let mut items = vec![json!(kind)];
    items.extend(nested(input));
    Value::Array(items)
}

fn nested(input: &mut Parser<'_, '_>) -> Vec<Value> {
    input
        .parse_nested_block(|block| Ok(component_values(block)))
        .unwrap_or_default()
}

fn next_value(input: &mut Parser<'_, '_>) -> Option<Vec<Value>> {
    let start = input.position();
    let token = input.next_including_whitespace().ok()?;
    let source = strip_comments(input.slice_from(start));
    Some(component_value(input, token, source))
}

fn component_values(input: &mut Parser<'_, '_>) -> Vec<Value> {
    let mut values = Vec::new();
    while let Some(value) = next_value(input) {
        values.extend(value);
    }
    values
}

struct Collect {
    nested_rules: bool,
}

impl<'i> RuleHandler<'i> for Collect {
    type AtPrelude = (String, Vec<Value>);
    type QualifiedPrelude = Vec<Value>;
    type Item = Value;

    fn at_rule_prelude(
        &mut self,
        name: Cow<'i, str>,
        input: &mut Parser<'i, '_>,
    ) -> ParseResult<Self::AtPrelude> {
        Ok((name.into_owned(), component_values(input)))
    }

    fn at_rule_block(
        &mut self,
        (name, prelude): Self::AtPrelude,
        input: &mut Parser<'i, '_>,
    ) -> ParseResult<Value> {
        Ok(json!(["at-rule", name, prelude, component_values(input)]))
    }

    fn at_rule_without_block(&mut self, (name, prelude): Self::AtPrelude) -> ParseResult<Value> {
        Ok(json!(["at-rule", name, prelude, null]))
    }

    fn qualified_rule_prelude(&mut self, input: &mut Parser<'i, '_>) -> ParseResult<Vec<Value>> {
        if self.nested_rules {
            Ok(component_values(input))
        } else {
            Err(ParseError::Invalid)
        }
    }

    fn qualified_rule_block(
        &mut self,
        prelude: Vec<Value>,
        input: &mut Parser<'i, '_>,
    ) -> ParseResult<Value> {
        Ok(json!(["qualified rule", prelude, component_values(input)]))
    }

    fn declaration(
        &mut self,
        name: Cow<'i, str>,
        input: &mut Parser<'i, '_>,
    ) -> ParseResult<Value> {
        let mut value = component_values(input);
        let space = json!(" ");
        let is_curly =
            |item: &Value| item.as_array().and_then(|items| items.first()) == Some(&json!("{}"));
        let significant: Vec<usize> = value
            .iter()
            .enumerate()
            .filter(|(_, item)| **item != space)
            .map(|(index, _)| index)
            .collect();
        if value.iter().any(is_curly) && significant.len() > 1 && !name.starts_with("--") {
            return Err(ParseError::Invalid);
        }
        let mut important = false;
        if let [.., bang, last] = significant[..]
            && value.get(bang) == Some(&json!("!"))
            && value
                .get(last)
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    items.first() == Some(&json!("ident"))
                        && items
                            .get(1)
                            .and_then(Value::as_str)
                            .is_some_and(|ident| ident.eq_ignore_ascii_case("important"))
                })
        {
            important = true;
            value.truncate(bang);
        }
        Ok(json!(["declaration", name, value, important]))
    }
}

fn results(items: impl Iterator<Item = ParseResult<Value>>) -> Value {
    Value::Array(
        items
            .map(|item| item.unwrap_or_else(|_| error("invalid")))
            .collect(),
    )
}

fn one_rule(input: &mut Parser<'_, '_>) -> Value {
    input.skip_whitespace();
    if input.is_exhausted() {
        return error("empty");
    }
    let rule =
        RuleListParser::new_for_nested_rules(input, &mut Collect { nested_rules: true }).next();
    input.skip_whitespace();
    match rule {
        Some(Ok(_)) if !input.is_exhausted() => error("extra-input"),
        Some(Ok(rule)) => rule,
        _ => error("invalid"),
    }
}

fn one_declaration(input: &mut Parser<'_, '_>) -> Value {
    input.skip_whitespace();
    if input.is_exhausted() {
        return error("empty");
    }
    let Ok(name) = input.expect_ident() else {
        return error("invalid");
    };
    if input.expect_token(Token::Colon).is_err() {
        return error("invalid");
    }
    Collect { nested_rules: true }
        .declaration(name, input)
        .unwrap_or_else(|_| error("invalid"))
}

fn one_component_value(input: &mut Parser<'_, '_>) -> Value {
    input.skip_whitespace();
    let Some(mut value) = next_value(input) else {
        return error("empty");
    };
    input.skip_whitespace();
    if !input.is_exhausted() || value.len() != 1 {
        return error("extra-input");
    }
    value.pop().unwrap_or(Value::Null)
}
