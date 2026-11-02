use std::borrow::Cow;

use web_atoms::{Atom, AtomTable};

use crate::media::MediaList;
use crate::parser::{Delimiters, ParseError, ParseResult, Parser};
use crate::properties::DeclarationBlock;
use crate::rules::{BlockContentsParser, RuleHandler, RuleListParser};
use crate::selectors::{Namespaces, SelectorList, SelectorParser};
use crate::tokenizer::{Token, Tokenizer, preprocess};
use crate::values::{Range, parse_percentage};

#[derive(Clone, Debug, PartialEq)]
pub struct LayerName(pub Box<[Atom]>);

#[derive(Clone, Debug, PartialEq)]
pub struct StyleRule {
    pub selectors: SelectorList,
    pub declarations: DeclarationBlock,
    pub rules: Vec<CssRule>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportRule {
    pub url: Box<str>,
    pub layer: Option<Option<LayerName>>,
    pub supported: bool,
    pub media: MediaList,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    pub offsets: Box<[f32]>,
    pub declarations: DeclarationBlock,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CssRule {
    Style(Box<StyleRule>),
    Media(Box<MediaList>, Vec<CssRule>),
    Supports(bool, Vec<CssRule>),
    Container(Option<Box<str>>, Box<str>, Vec<CssRule>),
    Layer(Option<LayerName>, Vec<CssRule>),
    LayerStatement(Box<[LayerName]>),
    Import(Box<ImportRule>),
    FontFace(Box<[(Box<str>, Box<str>)]>),
    Keyframes(Box<str>, Box<[Keyframe]>),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stylesheet {
    pub rules: Vec<CssRule>,
    pub namespaces: Namespaces,
}

const MAX_NESTING_EXPANSION: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Imports,
    Namespaces,
    Body,
}

pub enum AtPrelude {
    Media(MediaList),
    Supports(bool),
    Container(Option<Box<str>>, Box<str>),
    LayerBlock(Option<LayerName>),
    LayerStatement(Vec<LayerName>),
    Import(Box<ImportRule>),
    Namespace(Option<Atom>, Atom),
    FontFace,
    Keyframes(Box<str>),
    Ignored,
}

enum Item {
    Rule(CssRule),
    Declaration,
    Nothing,
}

struct RuleParser<'a> {
    atoms: &'a mut AtomTable,
    namespaces: &'a mut Namespaces,
    parent: Option<SelectorList>,
    expansion: usize,
    phase: Phase,
    declarations: DeclarationBlock,
}

fn parse_layer_name(input: &mut Parser<'_, '_>, atoms: &mut AtomTable) -> ParseResult<LayerName> {
    let mut parts = vec![atoms.intern(&input.expect_ident()?)];
    loop {
        let state = input.state();
        match input.next_including_whitespace() {
            Ok(Token::Delim('.')) => match input.next_including_whitespace()? {
                Token::Ident(part) => parts.push(atoms.intern(&part)),
                _ => return Err(ParseError::Invalid),
            },
            _ => {
                input.reset(&state);
                return Ok(LayerName(parts.into_boxed_slice()));
            }
        }
    }
}

fn rest_as_text<'i>(input: &mut Parser<'i, '_>) -> &'i str {
    input.skip_whitespace();
    let start = input.position();
    while input.next_including_whitespace().is_ok() {}
    input.slice_from(start).trim_end()
}

fn supports_condition(input: &mut Parser<'_, '_>, atoms: &mut AtomTable) -> ParseResult<bool> {
    if input.try_ident_matching("not") {
        return supports_in_parens(input, atoms).map(|v| !v);
    }
    let mut result = supports_in_parens(input, atoms)?;
    let mut mode = None;
    loop {
        let state = input.state();
        let Ok(ident) = input.expect_ident() else {
            input.reset(&state);
            return Ok(result);
        };
        let is_and = if ident.eq_ignore_ascii_case("and") {
            true
        } else if ident.eq_ignore_ascii_case("or") {
            false
        } else {
            return Err(ParseError::Invalid);
        };
        if mode.is_some_and(|m| m != is_and) {
            return Err(ParseError::Invalid);
        }
        mode = Some(is_and);
        let next = supports_in_parens(input, atoms)?;
        result = if is_and {
            result && next
        } else {
            result || next
        };
    }
}

fn supports_in_parens(input: &mut Parser<'_, '_>, atoms: &mut AtomTable) -> ParseResult<bool> {
    match input.next_token()? {
        Token::OpenParen => input.parse_nested_block(|p| {
            if let Ok(result) = p.try_parse(|p| {
                let result = supports_condition(p, atoms)?;
                p.expect_exhausted()?;
                Ok(result)
            }) {
                return Ok(result);
            }
            let declaration = p.try_parse(|p| {
                let name = p.expect_ident()?;
                p.expect_token(Token::Colon)?;
                let mut block = DeclarationBlock::default();
                block.parse_declaration(&name, p, atoms)
            });
            while p.next_including_whitespace().is_ok() {}
            Ok(declaration.is_ok())
        }),
        Token::Function(name) => {
            let is_selector = name.eq_ignore_ascii_case("selector");
            input.parse_nested_block(|p| {
                let supported = is_selector
                    && p.try_parse(|p| {
                        let namespaces = Namespaces::default();
                        SelectorParser {
                            atoms,
                            namespaces: &namespaces,
                            parent: None,
                        }
                        .parse_list(p)
                    })
                    .is_ok();
                while p.next_including_whitespace().is_ok() {}
                Ok(supported)
            })
        }
        _ => Err(ParseError::Invalid),
    }
}

fn keyframe_offsets(input: &mut Parser<'_, '_>) -> ParseResult<Box<[f32]>> {
    input
        .parse_comma_separated(|p| {
            if p.try_ident_matching("from") {
                return Ok(0.0);
            }
            if p.try_ident_matching("to") {
                return Ok(1.0);
            }
            let percent = parse_percentage(p, Range::NonNegative)?;
            if percent > 100.0 {
                return Err(ParseError::Invalid);
            }
            Ok(percent / 100.0)
        })
        .map(Vec::into_boxed_slice)
}

impl RuleParser<'_> {
    fn nested(&mut self, parent: Option<SelectorList>) -> RuleParser<'_> {
        let expansion = match (&parent, &self.parent) {
            (Some(list), Some(previous)) if !std::sync::Arc::ptr_eq(&list.0, &previous.0) => {
                self.expansion.saturating_mul(list.0.len().max(1))
            }
            (Some(list), None) => list.0.len().max(1),
            _ => self.expansion,
        };
        RuleParser {
            atoms: &mut *self.atoms,
            namespaces: &mut *self.namespaces,
            parent,
            expansion,
            phase: Phase::Body,
            declarations: DeclarationBlock::default(),
        }
    }

    fn group_rules(&mut self, input: &mut Parser<'_, '_>) -> Vec<CssRule> {
        match self.parent.clone() {
            None => {
                let mut nested = self.nested(None);
                RuleListParser::new_for_nested_rules(input, &mut nested)
                    .filter_map(|item| match item {
                        Ok(Item::Rule(rule)) => Some(rule),
                        _ => None,
                    })
                    .collect()
            }
            Some(parent) => {
                let (declarations, mut rules) = self.style_block(input, parent.clone());
                if !declarations.is_empty() {
                    rules.insert(
                        0,
                        CssRule::Style(Box::new(StyleRule {
                            selectors: parent,
                            declarations,
                            rules: vec![],
                        })),
                    );
                }
                rules
            }
        }
    }

    fn style_block(
        &mut self,
        input: &mut Parser<'_, '_>,
        selectors: SelectorList,
    ) -> (DeclarationBlock, Vec<CssRule>) {
        let mut nested = self.nested(Some(selectors));
        let rules = BlockContentsParser::new(input, &mut nested)
            .filter_map(|item| match item {
                Ok(Item::Rule(rule)) => Some(rule),
                _ => None,
            })
            .collect();
        (nested.declarations, rules)
    }

    fn import_prelude(&mut self, input: &mut Parser<'_, '_>) -> ParseResult<AtPrelude> {
        let url = input.expect_url_or_string()?.into();
        let layer = if input.try_ident_matching("layer") {
            Some(None)
        } else if input
            .try_parse(|p| p.expect_function_matching("layer"))
            .is_ok()
        {
            let atoms = &mut *self.atoms;
            Some(Some(input.parse_nested_block(|p| {
                let name = parse_layer_name(p, atoms)?;
                p.expect_exhausted()?;
                Ok(name)
            })?))
        } else {
            None
        };
        let supported = if input
            .try_parse(|p| p.expect_function_matching("supports"))
            .is_ok()
        {
            let atoms = &mut *self.atoms;
            input.parse_nested_block(|p| {
                if let Ok(result) = p.try_parse(|p| supports_condition(p, atoms)) {
                    return Ok(result);
                }
                let name = p.expect_ident()?;
                p.expect_token(Token::Colon)?;
                Ok(DeclarationBlock::default()
                    .parse_declaration(&name, p, atoms)
                    .is_ok())
            })?
        } else {
            true
        };
        let media = MediaList::parse(input);
        Ok(AtPrelude::Import(Box::new(ImportRule {
            url,
            layer,
            supported,
            media,
        })))
    }
}

impl<'i> RuleHandler<'i> for RuleParser<'_> {
    type AtPrelude = AtPrelude;
    type QualifiedPrelude = SelectorList;
    type Item = Item;

    fn at_rule_prelude(
        &mut self,
        name: Cow<'i, str>,
        input: &mut Parser<'i, '_>,
    ) -> ParseResult<AtPrelude> {
        let name = name.to_ascii_lowercase();
        let top_level = self.parent.is_none();
        let prelude = match name.as_str() {
            "charset" if top_level => AtPrelude::Ignored,
            "import" if top_level && self.phase <= Phase::Imports => self.import_prelude(input)?,
            "namespace" if top_level && self.phase <= Phase::Namespaces => {
                let prefix = input
                    .try_parse(|p| p.expect_ident())
                    .ok()
                    .map(|p| self.atoms.intern(&p));
                let url = self.atoms.intern(&input.expect_url_or_string()?);
                AtPrelude::Namespace(prefix, url)
            }
            "media" => AtPrelude::Media(MediaList::parse(input)),
            "supports" => AtPrelude::Supports(supports_condition(input, self.atoms)?),
            "container" => {
                let name = input
                    .try_parse(|p| {
                        let ident = p.expect_ident()?;
                        if ["not", "and", "or", "none"]
                            .iter()
                            .any(|k| ident.eq_ignore_ascii_case(k))
                        {
                            return Err(ParseError::Invalid);
                        }
                        Ok(Box::<str>::from(ident))
                    })
                    .ok();
                let condition = rest_as_text(input);
                if condition.is_empty() {
                    return Err(ParseError::Invalid);
                }
                AtPrelude::Container(name, condition.into())
            }
            "layer" => {
                input.skip_whitespace();
                if input.is_exhausted() {
                    AtPrelude::LayerBlock(None)
                } else {
                    let atoms = &mut *self.atoms;
                    let names = input.parse_comma_separated(|p| parse_layer_name(p, atoms))?;
                    AtPrelude::LayerStatement(names)
                }
            }
            "font-face" if top_level => {
                input.expect_exhausted()?;
                AtPrelude::FontFace
            }
            "keyframes" | "-webkit-keyframes" if top_level => {
                let name = match input.next_token()? {
                    Token::Ident(name) => {
                        if ["none", "initial", "inherit", "unset", "default"]
                            .iter()
                            .any(|k| name.eq_ignore_ascii_case(k))
                        {
                            return Err(ParseError::Invalid);
                        }
                        name
                    }
                    Token::String(name) => name,
                    _ => return Err(ParseError::Invalid),
                };
                input.expect_exhausted()?;
                AtPrelude::Keyframes(name.into())
            }
            _ => return Err(ParseError::Invalid),
        };
        let phase = match prelude {
            AtPrelude::Ignored => self.phase,
            AtPrelude::Import(_) => Phase::Imports,
            AtPrelude::Namespace(..) => Phase::Namespaces,
            AtPrelude::LayerStatement(_) => self.phase,
            _ => Phase::Body,
        };
        self.phase = self.phase.max(phase);
        Ok(prelude)
    }

    fn at_rule_block(
        &mut self,
        prelude: AtPrelude,
        input: &mut Parser<'i, '_>,
    ) -> ParseResult<Item> {
        let rule = match prelude {
            AtPrelude::Media(media) => CssRule::Media(Box::new(media), self.group_rules(input)),
            AtPrelude::Supports(enabled) => CssRule::Supports(enabled, self.group_rules(input)),
            AtPrelude::Container(name, condition) => {
                CssRule::Container(name, condition, self.group_rules(input))
            }
            AtPrelude::LayerBlock(name) => CssRule::Layer(name, self.group_rules(input)),
            AtPrelude::LayerStatement(mut names) if names.len() == 1 => {
                CssRule::Layer(names.pop(), self.group_rules(input))
            }
            AtPrelude::FontFace => {
                let mut descriptors = FontFaceDescriptors::default();
                for _ in BlockContentsParser::new(input, &mut descriptors) {}
                CssRule::FontFace(descriptors.0.into_boxed_slice())
            }
            AtPrelude::Keyframes(name) => {
                let mut keyframes = KeyframesParser {
                    atoms: &mut *self.atoms,
                };
                let frames: Vec<Keyframe> =
                    RuleListParser::new_for_nested_rules(input, &mut keyframes)
                        .filter_map(Result::ok)
                        .collect();
                CssRule::Keyframes(name, frames.into_boxed_slice())
            }
            _ => return Err(ParseError::Invalid),
        };
        Ok(Item::Rule(rule))
    }

    fn at_rule_without_block(&mut self, prelude: AtPrelude) -> ParseResult<Item> {
        Ok(match prelude {
            AtPrelude::Ignored => Item::Nothing,
            AtPrelude::Import(import) => Item::Rule(CssRule::Import(import)),
            AtPrelude::LayerStatement(names) => {
                Item::Rule(CssRule::LayerStatement(names.into_boxed_slice()))
            }
            AtPrelude::Namespace(prefix, url) => {
                match prefix {
                    Some(prefix) => self.namespaces.prefixes.push((prefix, url)),
                    None => self.namespaces.default = Some(url),
                }
                Item::Nothing
            }
            _ => return Err(ParseError::Invalid),
        })
    }

    fn qualified_rule_prelude(&mut self, input: &mut Parser<'i, '_>) -> ParseResult<SelectorList> {
        self.phase = Phase::Body;
        if self.expansion > MAX_NESTING_EXPANSION {
            return Err(ParseError::TooDeep);
        }
        SelectorParser {
            atoms: &mut *self.atoms,
            namespaces: &*self.namespaces,
            parent: self.parent.as_ref(),
        }
        .parse_list(input)
    }

    fn qualified_rule_block(
        &mut self,
        selectors: SelectorList,
        input: &mut Parser<'i, '_>,
    ) -> ParseResult<Item> {
        let (declarations, rules) = self.style_block(input, selectors.clone());
        Ok(Item::Rule(CssRule::Style(Box::new(StyleRule {
            selectors,
            declarations,
            rules,
        }))))
    }

    fn declaration(&mut self, name: Cow<'i, str>, input: &mut Parser<'i, '_>) -> ParseResult<Item> {
        if self.parent.is_none() {
            return Err(ParseError::Invalid);
        }
        self.declarations
            .parse_declaration(&name, input, self.atoms)?;
        Ok(Item::Declaration)
    }
}

#[derive(Default)]
struct FontFaceDescriptors(Vec<(Box<str>, Box<str>)>);

impl<'i> RuleHandler<'i> for FontFaceDescriptors {
    type AtPrelude = ();
    type QualifiedPrelude = ();
    type Item = ();

    fn declaration(&mut self, name: Cow<'i, str>, input: &mut Parser<'i, '_>) -> ParseResult<()> {
        let value = rest_as_text(input);
        if value.is_empty() {
            return Err(ParseError::Invalid);
        }
        self.0
            .push((name.to_ascii_lowercase().into(), value.into()));
        Ok(())
    }
}

struct KeyframesParser<'a> {
    atoms: &'a mut AtomTable,
}

impl<'i> RuleHandler<'i> for KeyframesParser<'_> {
    type AtPrelude = ();
    type QualifiedPrelude = Box<[f32]>;
    type Item = Keyframe;

    fn qualified_rule_prelude(&mut self, input: &mut Parser<'i, '_>) -> ParseResult<Box<[f32]>> {
        keyframe_offsets(input)
    }

    fn qualified_rule_block(
        &mut self,
        offsets: Box<[f32]>,
        input: &mut Parser<'i, '_>,
    ) -> ParseResult<Keyframe> {
        Ok(Keyframe {
            offsets,
            declarations: parse_declarations(input, self.atoms),
        })
    }
}

struct DeclarationsOnly<'a> {
    atoms: &'a mut AtomTable,
    block: DeclarationBlock,
}

impl<'i> RuleHandler<'i> for DeclarationsOnly<'_> {
    type AtPrelude = ();
    type QualifiedPrelude = ();
    type Item = ();

    fn declaration(&mut self, name: Cow<'i, str>, input: &mut Parser<'i, '_>) -> ParseResult<()> {
        self.block.parse_declaration(&name, input, self.atoms)
    }
}

fn parse_declarations(input: &mut Parser<'_, '_>, atoms: &mut AtomTable) -> DeclarationBlock {
    let mut handler = DeclarationsOnly {
        atoms,
        block: DeclarationBlock::default(),
    };
    for _ in BlockContentsParser::new(input, &mut handler) {}
    handler.block
}

pub fn parse_style_attribute(css: &str, atoms: &mut AtomTable) -> DeclarationBlock {
    let css = preprocess(css);
    let mut tokenizer = Tokenizer::new(&css);
    let mut input = Parser::new(&mut tokenizer);
    parse_declarations(&mut input, atoms)
}

pub fn parse_stylesheet(css: &str, atoms: &mut AtomTable) -> Stylesheet {
    let css = preprocess(css);
    let mut tokenizer = Tokenizer::new(&css);
    let mut input = Parser::new(&mut tokenizer);
    let mut namespaces = Namespaces::default();
    let mut handler = RuleParser {
        atoms,
        namespaces: &mut namespaces,
        parent: None,
        expansion: 1,
        phase: Phase::Imports,
        declarations: DeclarationBlock::default(),
    };
    let rules = RuleListParser::new_for_stylesheet(&mut input, &mut handler)
        .filter_map(|item| match item {
            Ok(Item::Rule(rule)) => Some(rule),
            _ => None,
        })
        .collect();
    Stylesheet { rules, namespaces }
}

pub fn parse_selector_list(
    css: &str,
    atoms: &mut AtomTable,
    namespaces: &Namespaces,
) -> ParseResult<SelectorList> {
    let css = preprocess(css);
    let mut tokenizer = Tokenizer::new(&css);
    let mut input = Parser::new(&mut tokenizer);
    input.parse_until_before(Delimiters::NONE, |p| {
        let list = SelectorParser {
            atoms,
            namespaces,
            parent: None,
        }
        .parse_list(p)?;
        p.expect_exhausted()?;
        Ok(list)
    })
}
