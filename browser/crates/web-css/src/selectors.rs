use std::sync::Arc;

use web_atoms::{Atom, AtomTable};

use crate::nth::parse_nth;
use crate::parser::{Delimiters, ParseError, ParseResult, Parser};
use crate::tokenizer::Token;

#[derive(Clone, Debug, PartialEq)]
pub struct SelectorList(pub Arc<[Selector]>);

#[derive(Clone, Debug, PartialEq)]
pub struct Selector {
    pub components: Box<[Component]>,
    pub specificity: u32,
    pub pseudo_element: Option<PseudoElement>,
    pub ancestor_atoms: [Option<Atom>; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
    NextSibling,
    LaterSibling,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamespaceConstraint {
    Any,
    None,
    Url(Atom),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Component {
    Combinator(Combinator),
    LocalName { name: Atom, lower: Atom },
    Namespace(NamespaceConstraint),
    Id(Atom),
    Class(Atom),
    Attribute(Box<AttributeSelector>),
    PseudoClass(PseudoClass),
    Nth(Box<NthSelector>),
    Lang(Box<[Box<str>]>),
    Dir(Direction),
    Not(SelectorList),
    Is(SelectorList),
    Where(SelectorList),
    Has(SelectorList),
    RelativeAnchor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeOperator {
    Equal,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeCase {
    DocumentDefault,
    AsciiInsensitive,
    Sensitive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttributeSelector {
    pub name: Atom,
    pub lower_name: Atom,
    pub namespace: Option<NamespaceConstraint>,
    pub operation: Option<(AttributeOperator, Box<str>)>,
    pub case: AttributeCase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NthKind {
    Child,
    LastChild,
    OfType,
    LastOfType,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NthSelector {
    pub kind: NthKind,
    pub a: i32,
    pub b: i32,
    pub of: Option<SelectorList>,
}

impl NthSelector {
    pub fn matches_index(&self, index: i32) -> bool {
        let offset = i64::from(index) - i64::from(self.b);
        match self.a {
            0 => offset == 0,
            a => offset % i64::from(a) == 0 && offset / i64::from(a) >= 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Ltr,
    Rtl,
}

macro_rules! keyword_enum {
    ($name:ident { $($variant:ident = $text:literal,)* }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name { $($variant,)* }

        impl $name {
            pub fn from_name(name: &str) -> Option<Self> {
                $(if name.eq_ignore_ascii_case($text) { return Some($name::$variant); })*
                None
            }

            pub fn name(self) -> &'static str {
                match self { $($name::$variant => $text,)* }
            }
        }
    };
}

keyword_enum!(PseudoClass {
    Active = "active",
    AnyLink = "any-link",
    Autofill = "autofill",
    Checked = "checked",
    Default = "default",
    Defined = "defined",
    Disabled = "disabled",
    Empty = "empty",
    Enabled = "enabled",
    FirstChild = "first-child",
    FirstOfType = "first-of-type",
    Focus = "focus",
    FocusVisible = "focus-visible",
    FocusWithin = "focus-within",
    Fullscreen = "fullscreen",
    Hover = "hover",
    InRange = "in-range",
    Indeterminate = "indeterminate",
    Invalid = "invalid",
    LastChild = "last-child",
    LastOfType = "last-of-type",
    Link = "link",
    Modal = "modal",
    OnlyChild = "only-child",
    OnlyOfType = "only-of-type",
    Open = "open",
    Optional = "optional",
    OutOfRange = "out-of-range",
    PlaceholderShown = "placeholder-shown",
    PopoverOpen = "popover-open",
    ReadOnly = "read-only",
    ReadWrite = "read-write",
    Required = "required",
    Root = "root",
    Scope = "scope",
    Target = "target",
    UserInvalid = "user-invalid",
    UserValid = "user-valid",
    Valid = "valid",
    Visited = "visited",
});

keyword_enum!(PseudoElement {
    After = "after",
    Backdrop = "backdrop",
    Before = "before",
    FileSelectorButton = "file-selector-button",
    FirstLetter = "first-letter",
    FirstLine = "first-line",
    Marker = "marker",
    Placeholder = "placeholder",
    Selection = "selection",
    WebkitUnknown = "-webkit-",
});

impl PseudoElement {
    fn parse(name: &str) -> Option<PseudoElement> {
        if name.len() > 8
            && name
                .get(..8)
                .is_some_and(|p| p.eq_ignore_ascii_case("-webkit-"))
        {
            return Some(PseudoElement::WebkitUnknown);
        }
        PseudoElement::from_name(name).filter(|p| *p != PseudoElement::WebkitUnknown)
    }

    fn legacy_single_colon(self) -> bool {
        matches!(
            self,
            PseudoElement::Before
                | PseudoElement::After
                | PseudoElement::FirstLine
                | PseudoElement::FirstLetter
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Namespaces {
    pub default: Option<Atom>,
    pub prefixes: Vec<(Atom, Atom)>,
}

impl Namespaces {
    fn lookup(&self, prefix: Atom) -> Option<Atom> {
        self.prefixes
            .iter()
            .rev()
            .find(|(p, _)| *p == prefix)
            .map(|(_, url)| *url)
    }
}

pub struct SelectorParser<'a> {
    pub atoms: &'a mut AtomTable,
    pub namespaces: &'a Namespaces,
    pub parent: Option<&'a SelectorList>,
}

const ID_WEIGHT: u32 = 1 << 20;
const CLASS_WEIGHT: u32 = 1 << 10;
const TYPE_WEIGHT: u32 = 1;

fn add_specificity(a: u32, b: u32) -> u32 {
    let field = |v: u32, shift: u32| (v >> shift) & 0x3ff;
    let sum = |shift: u32| (field(a, shift) + field(b, shift)).min(0x3ff) << shift;
    sum(20) | sum(10) | sum(0)
}

impl SelectorList {
    pub fn max_specificity(&self) -> u32 {
        self.0.iter().map(|s| s.specificity).max().unwrap_or(0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListMode {
    Strict,
    Forgiving,
    Relative,
    Nested,
}

struct Builder {
    compounds: Vec<(Vec<Component>, Option<Combinator>)>,
    specificity: u32,
    pseudo_element: Option<PseudoElement>,
    has_nesting: bool,
}

impl<'a> SelectorParser<'a> {
    pub fn parse_list(&mut self, input: &mut Parser<'_, '_>) -> ParseResult<SelectorList> {
        let mode = if self.parent.is_some() {
            ListMode::Nested
        } else {
            ListMode::Strict
        };
        self.parse_list_with(input, mode)
    }

    fn parse_list_with(
        &mut self,
        input: &mut Parser<'_, '_>,
        mode: ListMode,
    ) -> ParseResult<SelectorList> {
        let mut selectors = vec![];
        loop {
            let selector = input.parse_until_before(Delimiters::COMMA, |p| {
                let selector = self.parse_complex(p, mode)?;
                p.expect_exhausted()?;
                Ok(selector)
            });
            match selector {
                Ok(selector) => selectors.push(selector),
                Err(error) if mode != ListMode::Forgiving => return Err(error),
                Err(_) => {}
            }
            match input.next_token() {
                Err(_) => break,
                Ok(Token::Comma) => {}
                Ok(_) => return Err(ParseError::Unexpected),
            }
        }
        if selectors.is_empty() && mode != ListMode::Forgiving {
            return Err(ParseError::Invalid);
        }
        Ok(SelectorList(selectors.into()))
    }

    fn parse_complex(
        &mut self,
        input: &mut Parser<'_, '_>,
        mode: ListMode,
    ) -> ParseResult<Selector> {
        let mut builder = Builder {
            compounds: vec![],
            specificity: 0,
            pseudo_element: None,
            has_nesting: false,
        };
        input.skip_whitespace();
        let leading = self.parse_combinator(input)?;
        let relative = matches!(mode, ListMode::Relative | ListMode::Nested);
        match (leading, mode) {
            (Some(_), _) if !relative => return Err(ParseError::Invalid),
            (Some(combinator), _) => builder.compounds.push((vec![], Some(combinator))),
            (None, ListMode::Relative) => builder
                .compounds
                .push((vec![], Some(Combinator::Descendant))),
            _ => {}
        }
        loop {
            let compound = self.parse_compound(input, &mut builder)?;
            if compound.is_empty() {
                return Err(ParseError::Invalid);
            }
            let combinator = self.parse_combinator(input)?;
            builder.compounds.push((compound, combinator));
            match combinator {
                None => break,
                Some(_) if builder.pseudo_element.is_some() => return Err(ParseError::Invalid),
                Some(_) => {}
            }
        }
        if mode == ListMode::Relative {
            if let Some((first, _)) = builder.compounds.first_mut() {
                first.push(Component::RelativeAnchor);
            }
        } else if mode == ListMode::Nested && !builder.has_nesting {
            match builder.compounds.first_mut() {
                Some((first, Some(_))) if first.is_empty() => {}
                _ => builder
                    .compounds
                    .insert(0, (vec![], Some(Combinator::Descendant))),
            }
            if let Some((first, _)) = builder.compounds.first_mut() {
                first.push(self.nesting_component(&mut builder.specificity));
            }
        }
        if let Some((first, _)) = builder.compounds.first_mut()
            && first.is_empty()
        {
            first.push(self.nesting_component(&mut builder.specificity));
        }
        Ok(builder.finish())
    }

    fn nesting_component(&self, specificity: &mut u32) -> Component {
        match self.parent {
            Some(parent) => {
                *specificity = add_specificity(*specificity, parent.max_specificity());
                Component::Is(parent.clone())
            }
            None => Component::PseudoClass(PseudoClass::Scope),
        }
    }

    fn parse_combinator(&mut self, input: &mut Parser<'_, '_>) -> ParseResult<Option<Combinator>> {
        let mut saw_whitespace = false;
        loop {
            let state = input.state();
            match input.next_including_whitespace() {
                Ok(Token::Whitespace) => saw_whitespace = true,
                Ok(Token::Delim(c @ ('>' | '+' | '~'))) => {
                    input.skip_whitespace();
                    return Ok(Some(match c {
                        '>' => Combinator::Child,
                        '+' => Combinator::NextSibling,
                        _ => Combinator::LaterSibling,
                    }));
                }
                Ok(_) => {
                    input.reset(&state);
                    return Ok(saw_whitespace.then_some(Combinator::Descendant));
                }
                Err(_) => {
                    input.reset(&state);
                    return Ok(None);
                }
            }
        }
    }

    fn parse_compound(
        &mut self,
        input: &mut Parser<'_, '_>,
        builder: &mut Builder,
    ) -> ParseResult<Vec<Component>> {
        let mut compound = vec![];
        self.parse_type_selector(input, &mut compound, &mut builder.specificity)?;
        loop {
            let state = input.state();
            let Ok(token) = input.next_including_whitespace() else {
                break;
            };
            if builder.pseudo_element.is_some() && token != Token::Colon {
                input.reset(&state);
                break;
            }
            match token {
                Token::IdHash(id) => {
                    compound.push(Component::Id(self.atoms.intern(&id)));
                    builder.specificity = add_specificity(builder.specificity, ID_WEIGHT);
                }
                Token::Delim('.') => match input.next_including_whitespace()? {
                    Token::Ident(class) => {
                        compound.push(Component::Class(self.atoms.intern(&class)));
                        builder.specificity = add_specificity(builder.specificity, CLASS_WEIGHT);
                    }
                    _ => return Err(ParseError::Invalid),
                },
                Token::OpenSquare => {
                    let attribute = input.parse_nested_block(|p| self.parse_attribute(p))?;
                    compound.push(Component::Attribute(Box::new(attribute)));
                    builder.specificity = add_specificity(builder.specificity, CLASS_WEIGHT);
                }
                Token::Colon => {
                    if builder.pseudo_element.is_some() {
                        return Err(ParseError::Invalid);
                    }
                    let double = {
                        let state = input.state();
                        let double = input.next_including_whitespace() == Ok(Token::Colon);
                        if !double {
                            input.reset(&state);
                        }
                        double
                    };
                    if double {
                        builder.pseudo_element = Some(self.parse_pseudo_element(input)?);
                        builder.specificity = add_specificity(builder.specificity, TYPE_WEIGHT);
                    } else {
                        self.parse_pseudo_class(input, &mut compound, builder)?;
                    }
                }
                Token::Delim('&') => {
                    builder.has_nesting = true;
                    let parent = self.nesting_component(&mut builder.specificity);
                    compound.push(parent);
                }
                _ => {
                    input.reset(&state);
                    break;
                }
            }
        }
        if compound.is_empty() && builder.pseudo_element.is_some() {
            compound.push(Component::Namespace(NamespaceConstraint::Any));
        }
        Ok(compound)
    }

    fn parse_namespace_prefix(
        &mut self,
        input: &mut Parser<'_, '_>,
    ) -> ParseResult<Option<NamespaceConstraint>> {
        let state = input.state();
        let prefix = match input.next_including_whitespace() {
            Ok(Token::Ident(prefix)) => Some(Some(self.atoms.intern(&prefix))),
            Ok(Token::Delim('*')) => Some(None),
            Ok(Token::Delim('|')) => {
                return Ok(Some(NamespaceConstraint::None));
            }
            _ => None,
        };
        if let Some(prefix) = prefix
            && input.next_including_whitespace() == Ok(Token::Delim('|'))
        {
            let after = input.state();
            let continues = matches!(
                input.next_including_whitespace(),
                Ok(Token::Ident(_) | Token::Delim('*'))
            );
            input.reset(&after);
            if continues {
                return match prefix {
                    None => Ok(Some(NamespaceConstraint::Any)),
                    Some(prefix) => self
                        .namespaces
                        .lookup(prefix)
                        .map(|url| Some(NamespaceConstraint::Url(url)))
                        .ok_or(ParseError::Invalid),
                };
            }
        }
        input.reset(&state);
        Ok(None)
    }

    fn parse_type_selector(
        &mut self,
        input: &mut Parser<'_, '_>,
        compound: &mut Vec<Component>,
        specificity: &mut u32,
    ) -> ParseResult<()> {
        let explicit = self.parse_namespace_prefix(input)?;
        let state = input.state();
        let name = match input.next_including_whitespace() {
            Ok(Token::Ident(name)) => Some(Some(name)),
            Ok(Token::Delim('*')) => Some(None),
            _ => None,
        };
        let Some(name) = name else {
            input.reset(&state);
            if explicit.is_some() {
                return Err(ParseError::Invalid);
            }
            if let Some(url) = self.namespaces.default {
                compound.push(Component::Namespace(NamespaceConstraint::Url(url)));
            }
            return Ok(());
        };
        let namespace = explicit.or(self.namespaces.default.map(NamespaceConstraint::Url));
        if let Some(namespace) = namespace.filter(|ns| *ns != NamespaceConstraint::Any) {
            compound.push(Component::Namespace(namespace));
        }
        match name {
            Some(name) => {
                let lower = self.atoms.intern(&name.to_ascii_lowercase());
                compound.push(Component::LocalName {
                    name: self.atoms.intern(&name),
                    lower,
                });
                *specificity = add_specificity(*specificity, TYPE_WEIGHT);
            }
            None if compound.is_empty() => {
                compound.push(Component::Namespace(NamespaceConstraint::Any));
            }
            None => {}
        }
        Ok(())
    }

    fn parse_attribute(&mut self, input: &mut Parser<'_, '_>) -> ParseResult<AttributeSelector> {
        input.skip_whitespace();
        let explicit = self.parse_namespace_prefix(input)?;
        let name = match input.next_including_whitespace()? {
            Token::Ident(name) => name,
            _ => return Err(ParseError::Invalid),
        };
        let lower_name = self.atoms.intern(&name.to_ascii_lowercase());
        let name = self.atoms.intern(&name);
        let operator = match input.next_token() {
            Err(_) => {
                return Ok(AttributeSelector {
                    name,
                    lower_name,
                    namespace: explicit,
                    operation: None,
                    case: AttributeCase::DocumentDefault,
                });
            }
            Ok(Token::Delim('=')) => AttributeOperator::Equal,
            Ok(Token::IncludeMatch) => AttributeOperator::Includes,
            Ok(Token::DashMatch) => AttributeOperator::DashMatch,
            Ok(Token::PrefixMatch) => AttributeOperator::Prefix,
            Ok(Token::SuffixMatch) => AttributeOperator::Suffix,
            Ok(Token::SubstringMatch) => AttributeOperator::Substring,
            Ok(Token::Delim(c @ ('~' | '|' | '^' | '$' | '*'))) => {
                input.expect_delim('=')?;
                match c {
                    '~' => AttributeOperator::Includes,
                    '|' => AttributeOperator::DashMatch,
                    '^' => AttributeOperator::Prefix,
                    '$' => AttributeOperator::Suffix,
                    _ => AttributeOperator::Substring,
                }
            }
            Ok(_) => return Err(ParseError::Invalid),
        };
        let value: Box<str> = match input.next_token()? {
            Token::Ident(value) | Token::String(value) => value.into(),
            _ => return Err(ParseError::Invalid),
        };
        let case = match input.next_token() {
            Err(_) => AttributeCase::DocumentDefault,
            Ok(Token::Ident(flag)) if flag.eq_ignore_ascii_case("i") => {
                AttributeCase::AsciiInsensitive
            }
            Ok(Token::Ident(flag)) if flag.eq_ignore_ascii_case("s") => AttributeCase::Sensitive,
            Ok(_) => return Err(ParseError::Invalid),
        };
        input.expect_exhausted()?;
        Ok(AttributeSelector {
            name,
            lower_name,
            namespace: explicit,
            operation: Some((operator, value)),
            case,
        })
    }

    fn parse_pseudo_element(&mut self, input: &mut Parser<'_, '_>) -> ParseResult<PseudoElement> {
        match input.next_including_whitespace()? {
            Token::Ident(name) => PseudoElement::parse(&name).ok_or(ParseError::Invalid),
            _ => Err(ParseError::Invalid),
        }
    }

    fn parse_pseudo_class(
        &mut self,
        input: &mut Parser<'_, '_>,
        compound: &mut Vec<Component>,
        builder: &mut Builder,
    ) -> ParseResult<()> {
        match input.next_including_whitespace()? {
            Token::Ident(name) => {
                if let Some(pseudo) =
                    PseudoElement::from_name(&name).filter(|p| p.legacy_single_colon())
                {
                    builder.pseudo_element = Some(pseudo);
                    builder.specificity = add_specificity(builder.specificity, TYPE_WEIGHT);
                    return Ok(());
                }
                let pseudo = PseudoClass::from_name(&name).ok_or(ParseError::Invalid)?;
                let weight = if pseudo == PseudoClass::Scope {
                    0
                } else {
                    CLASS_WEIGHT
                };
                builder.specificity = add_specificity(builder.specificity, weight);
                compound.push(Component::PseudoClass(pseudo));
                Ok(())
            }
            Token::Function(name) => {
                let name = name.to_ascii_lowercase();
                let (component, specificity) =
                    input.parse_nested_block(|p| self.parse_functional_pseudo(&name, p))?;
                builder.specificity = add_specificity(builder.specificity, specificity);
                compound.push(component);
                Ok(())
            }
            _ => Err(ParseError::Invalid),
        }
    }

    fn parse_functional_pseudo(
        &mut self,
        name: &str,
        input: &mut Parser<'_, '_>,
    ) -> ParseResult<(Component, u32)> {
        let nth_kind = match name {
            "nth-child" => Some(NthKind::Child),
            "nth-last-child" => Some(NthKind::LastChild),
            "nth-of-type" => Some(NthKind::OfType),
            "nth-last-of-type" => Some(NthKind::LastOfType),
            _ => None,
        };
        if let Some(kind) = nth_kind {
            let (a, b) = parse_nth(input)?;
            let mut specificity = CLASS_WEIGHT;
            let of = if matches!(kind, NthKind::Child | NthKind::LastChild)
                && input.try_ident_matching("of")
            {
                let list = self.parse_list_with(input, ListMode::Strict)?;
                specificity = add_specificity(specificity, list.max_specificity());
                Some(list)
            } else {
                None
            };
            input.expect_exhausted()?;
            return Ok((
                Component::Nth(Box::new(NthSelector { kind, a, b, of })),
                specificity,
            ));
        }
        match name {
            "not" | "is" | "matches" | "-webkit-any" | "where" => {
                let mode = if name == "not" {
                    ListMode::Strict
                } else {
                    ListMode::Forgiving
                };
                let list = self.parse_list_with(input, mode)?;
                let specificity = list.max_specificity();
                Ok(match name {
                    "not" => (Component::Not(list), specificity),
                    "where" => (Component::Where(list), 0),
                    _ => (Component::Is(list), specificity),
                })
            }
            "has" => {
                let list = self.parse_list_with(input, ListMode::Relative)?;
                let specificity = list.max_specificity();
                Ok((Component::Has(list), specificity))
            }
            "lang" => {
                let ranges = input.parse_comma_separated(|p| match p.next_token()? {
                    Token::Ident(value) | Token::String(value) => Ok(Box::<str>::from(value)),
                    _ => Err(ParseError::Invalid),
                })?;
                Ok((Component::Lang(ranges.into_boxed_slice()), CLASS_WEIGHT))
            }
            "dir" => {
                let direction = match input.expect_ident()? {
                    d if d.eq_ignore_ascii_case("ltr") => Direction::Ltr,
                    d if d.eq_ignore_ascii_case("rtl") => Direction::Rtl,
                    _ => return Err(ParseError::Invalid),
                };
                input.expect_exhausted()?;
                Ok((Component::Dir(direction), CLASS_WEIGHT))
            }
            _ => Err(ParseError::Invalid),
        }
    }
}

impl Builder {
    fn finish(self) -> Selector {
        let mut components = Vec::new();
        let mut ancestor_atoms = [None; 4];
        let mut ancestor_count = 0;
        for (index, (compound, combinator)) in self.compounds.iter().enumerate().rev() {
            if let Some(combinator) = combinator
                && index + 1 < self.compounds.len()
            {
                components.push(Component::Combinator(*combinator));
            }
            if matches!(combinator, Some(Combinator::Descendant | Combinator::Child))
                && index + 1 < self.compounds.len()
            {
                for component in compound {
                    let atom = match component {
                        Component::Id(atom) | Component::Class(atom) => Some(*atom),
                        Component::LocalName { lower, .. } => Some(*lower),
                        _ => None,
                    };
                    if let Some(atom) = atom
                        && ancestor_count < ancestor_atoms.len()
                    {
                        ancestor_atoms[ancestor_count] = Some(atom);
                        ancestor_count += 1;
                    }
                }
            }
            components.extend(compound.iter().cloned());
        }
        Selector {
            components: components.into_boxed_slice(),
            specificity: self.specificity,
            pseudo_element: self.pseudo_element,
            ancestor_atoms,
        }
    }
}
