use web_atoms::{Atom, Namespace};
use web_css::selectors::{
    AttributeCase, AttributeOperator, AttributeSelector, Combinator, Component, Direction,
    NamespaceConstraint, NthKind, NthSelector, PseudoClass, Selector, SelectorList,
};
use web_dom::{Document, NodeId, NodeKind};

pub(crate) struct Context<'a> {
    pub(crate) document: &'a Document,
    pub(crate) namespaces: &'a [(Atom, Namespace)],
    pub(crate) anchor: Option<NodeId>,
}

pub(crate) fn matches(context: &Context<'_>, selector: &Selector, element: NodeId) -> bool {
    selector.pseudo_element.is_none() && matches_components(context, &selector.components, element)
}

pub(crate) fn matches_pseudo_element(
    context: &Context<'_>,
    selector: &Selector,
    element: NodeId,
    pseudo: web_css::selectors::PseudoElement,
) -> bool {
    selector.pseudo_element == Some(pseudo)
        && matches_components(context, &selector.components, element)
}

pub(crate) fn matches_list(context: &Context<'_>, list: &SelectorList, element: NodeId) -> bool {
    list.0
        .iter()
        .any(|selector| matches_components(context, &selector.components, element))
}

fn matches_components(context: &Context<'_>, components: &[Component], element: NodeId) -> bool {
    let split = components
        .iter()
        .position(|component| matches!(component, Component::Combinator(_)))
        .unwrap_or(components.len());
    let (compound, rest) = components.split_at(split);
    if !compound
        .iter()
        .all(|component| matches_simple(context, component, element))
    {
        return false;
    }
    let Some((Component::Combinator(combinator), rest)) = rest.split_first() else {
        return true;
    };
    let document = context.document;
    match combinator {
        Combinator::Child => parent_element(document, element)
            .is_some_and(|parent| matches_components(context, rest, parent)),
        Combinator::Descendant => {
            std::iter::successors(parent_element(document, element), |&node| {
                parent_element(document, node)
            })
            .any(|ancestor| matches_components(context, rest, ancestor))
        }
        Combinator::NextSibling => previous_element(document, element)
            .is_some_and(|sibling| matches_components(context, rest, sibling)),
        Combinator::LaterSibling => {
            std::iter::successors(previous_element(document, element), |&node| {
                previous_element(document, node)
            })
            .any(|sibling| matches_components(context, rest, sibling))
        }
    }
}

pub(crate) fn parent_element(document: &Document, node: NodeId) -> Option<NodeId> {
    document
        .parent(node)
        .filter(|&parent| document.kind(parent) == Some(NodeKind::Element))
}

fn is_element(document: &Document, node: NodeId) -> bool {
    document.kind(node) == Some(NodeKind::Element)
}

fn previous_element(document: &Document, node: NodeId) -> Option<NodeId> {
    std::iter::successors(document.previous_sibling(node), |&sibling| {
        document.previous_sibling(sibling)
    })
    .find(|&sibling| is_element(document, sibling))
}

fn next_element(document: &Document, node: NodeId) -> Option<NodeId> {
    std::iter::successors(document.next_sibling(node), |&sibling| {
        document.next_sibling(sibling)
    })
    .find(|&sibling| is_element(document, sibling))
}

fn matches_simple(context: &Context<'_>, component: &Component, element: NodeId) -> bool {
    let document = context.document;
    let Some((namespace, local)) = document.element_name(element) else {
        return false;
    };
    match component {
        Component::Combinator(_) => false,
        Component::LocalName { name, lower } => {
            if namespace == Namespace::Html {
                local == *lower
            } else {
                local == *name
            }
        }
        Component::Namespace(constraint) => match constraint {
            NamespaceConstraint::Any => true,
            NamespaceConstraint::None => false,
            NamespaceConstraint::Url(url) => context
                .namespaces
                .iter()
                .any(|&(atom, known)| atom == *url && known == namespace),
        },
        Component::Id(id) => document.element_id(element) == Some(*id),
        Component::Class(class) => document.classes(element).contains(class),
        Component::Attribute(attribute) => matches_attribute(document, attribute, element),
        Component::PseudoClass(pseudo) => matches_pseudo(context, *pseudo, element),
        Component::Nth(nth) => matches_nth(context, nth, element),
        Component::Lang(ranges) => {
            let lang = document
                .ancestors(element)
                .find_map(|node| document.attribute(node, Atom::LANG))
                .unwrap_or_default();
            ranges.iter().any(|range| {
                lang.len() >= range.len()
                    && lang
                        .get(..range.len())
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(range))
                    && (lang.len() == range.len()
                        || lang.as_bytes().get(range.len()) == Some(&b'-'))
            })
        }
        Component::Dir(direction) => {
            let rtl = document
                .ancestors(element)
                .find_map(|node| document.attribute(node, Atom::DIR))
                .is_some_and(|dir| dir.eq_ignore_ascii_case("rtl"));
            rtl == (*direction == Direction::Rtl)
        }
        Component::Not(list) => !matches_list(context, list, element),
        Component::Is(list) | Component::Where(list) => matches_list(context, list, element),
        Component::Has(list) => matches_has(context, list, element),
        Component::RelativeAnchor => context.anchor == Some(element),
    }
}

fn matches_has(context: &Context<'_>, list: &SelectorList, element: NodeId) -> bool {
    let document = context.document;
    let inner = Context {
        document,
        namespaces: context.namespaces,
        anchor: Some(element),
    };
    list.0.iter().any(|selector| {
        let siblings = selector
            .components
            .iter()
            .rev()
            .nth(1)
            .is_some_and(|component| {
                matches!(
                    component,
                    Component::Combinator(Combinator::NextSibling | Combinator::LaterSibling)
                )
            });
        if siblings {
            std::iter::successors(next_element(document, element), |&node| {
                next_element(document, node)
            })
            .any(|candidate| {
                matches_components(&inner, &selector.components, candidate)
                    || descendants(document, candidate)
                        .any(|node| matches_components(&inner, &selector.components, node))
            })
        } else {
            descendants(document, element)
                .any(|node| matches_components(&inner, &selector.components, node))
        }
    })
}

fn descendants(document: &Document, root: NodeId) -> impl Iterator<Item = NodeId> + '_ {
    let mut stack: Vec<NodeId> = document.children(root).collect();
    stack.reverse();
    std::iter::from_fn(move || {
        while let Some(node) = stack.pop() {
            let start = stack.len();
            stack.extend(document.children(node));
            if let Some(added) = stack.get_mut(start..) {
                added.reverse();
            }
            if is_element(document, node) {
                return Some(node);
            }
        }
        None
    })
}

fn matches_attribute(document: &Document, selector: &AttributeSelector, element: NodeId) -> bool {
    let html = document
        .element_name(element)
        .map(|(namespace, _)| namespace)
        == Some(Namespace::Html);
    let name = if html {
        selector.lower_name
    } else {
        selector.name
    };
    let mut values = document.attributes(element).filter(|(qualified, _)| {
        qualified.local == name
            && match selector.namespace {
                None => qualified.ns == Namespace::None,
                Some(NamespaceConstraint::Any) => true,
                Some(NamespaceConstraint::None) => qualified.ns == Namespace::None,
                Some(NamespaceConstraint::Url(_)) => qualified.ns != Namespace::None,
            }
    });
    let Some((operator, expected)) = &selector.operation else {
        return values.next().is_some();
    };
    let insensitive = match selector.case {
        AttributeCase::AsciiInsensitive => true,
        AttributeCase::Sensitive => false,
        AttributeCase::DocumentDefault => {
            html && case_insensitive_attribute(selector.lower_name, document)
        }
    };
    let equal = |a: &str, b: &str| {
        if insensitive {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    values.any(|(_, value)| match operator {
        AttributeOperator::Equal => equal(value, expected),
        AttributeOperator::Includes => {
            !expected.is_empty()
                && !expected.contains(char::is_whitespace)
                && value
                    .split_ascii_whitespace()
                    .any(|word| equal(word, expected))
        }
        AttributeOperator::DashMatch => {
            equal(value, expected)
                || (value.len() > expected.len()
                    && value.as_bytes().get(expected.len()) == Some(&b'-')
                    && value
                        .get(..expected.len())
                        .is_some_and(|prefix| equal(prefix, expected)))
        }
        AttributeOperator::Prefix => {
            !expected.is_empty()
                && value
                    .get(..expected.len())
                    .is_some_and(|prefix| equal(prefix, expected))
        }
        AttributeOperator::Suffix => {
            !expected.is_empty()
                && value.len() >= expected.len()
                && value
                    .get(value.len() - expected.len()..)
                    .is_some_and(|suffix| equal(suffix, expected))
        }
        AttributeOperator::Substring => {
            !expected.is_empty()
                && if insensitive {
                    value
                        .to_ascii_lowercase()
                        .contains(&expected.to_ascii_lowercase())
                } else {
                    value.contains(&**expected)
                }
        }
    })
}

fn case_insensitive_attribute(name: Atom, document: &Document) -> bool {
    const NAMES: &[&str] = &[
        "accept",
        "accept-charset",
        "align",
        "alink",
        "axis",
        "bgcolor",
        "charset",
        "checked",
        "clear",
        "codetype",
        "color",
        "compact",
        "declare",
        "defer",
        "dir",
        "direction",
        "disabled",
        "enctype",
        "face",
        "frame",
        "hreflang",
        "http-equiv",
        "lang",
        "language",
        "link",
        "media",
        "method",
        "multiple",
        "nohref",
        "noresize",
        "noshade",
        "nowrap",
        "readonly",
        "rel",
        "rev",
        "rules",
        "scope",
        "scrolling",
        "selected",
        "shape",
        "target",
        "text",
        "type",
        "valign",
        "valuetype",
        "vlink",
    ];
    NAMES.contains(&document.atoms().name(name))
}

fn matches_pseudo(context: &Context<'_>, pseudo: PseudoClass, element: NodeId) -> bool {
    let document = context.document;
    let html_name = document.html_element_name(element);
    let is_link = || {
        matches!(html_name, Some(Atom::A | Atom::AREA))
            && document.attribute(element, Atom::HREF).is_some()
    };
    match pseudo {
        PseudoClass::AnyLink | PseudoClass::Link => is_link(),
        PseudoClass::Hover => document.is_hovered(element),
        PseudoClass::Root => document.parent(element) == Some(NodeId::DOCUMENT),
        PseudoClass::Scope => context.anchor.map_or(
            document.parent(element) == Some(NodeId::DOCUMENT),
            |anchor| anchor == element,
        ),
        PseudoClass::Empty => document
            .children(element)
            .all(|child| match document.kind(child) {
                Some(NodeKind::Element) => false,
                Some(NodeKind::Text) => document.text(child).is_some_and(str::is_empty),
                _ => true,
            }),
        PseudoClass::FirstChild => previous_element(document, element).is_none(),
        PseudoClass::LastChild => next_element(document, element).is_none(),
        PseudoClass::OnlyChild => {
            previous_element(document, element).is_none()
                && next_element(document, element).is_none()
        }
        PseudoClass::FirstOfType | PseudoClass::LastOfType | PseudoClass::OnlyOfType => {
            let name = document.element_name(element);
            let same = |node: &NodeId| document.element_name(*node) == name;
            let before = std::iter::successors(previous_element(document, element), |&node| {
                previous_element(document, node)
            })
            .any(|node| same(&node));
            let after = std::iter::successors(next_element(document, element), |&node| {
                next_element(document, node)
            })
            .any(|node| same(&node));
            match pseudo {
                PseudoClass::FirstOfType => !before,
                PseudoClass::LastOfType => !after,
                _ => !before && !after,
            }
        }
        PseudoClass::Defined => true,
        PseudoClass::Enabled => {
            matches!(
                html_name,
                Some(Atom::BUTTON | Atom::INPUT | Atom::SELECT | Atom::TEXTAREA | Atom::OPTION)
            ) && document.attribute(element, Atom::DISABLED).is_none()
        }
        PseudoClass::Disabled => document.attribute(element, Atom::DISABLED).is_some(),
        PseudoClass::Checked => {
            document.attribute(element, Atom::CHECKED).is_some()
                || document.attribute(element, Atom::SELECTED).is_some()
        }
        PseudoClass::Open => document.attribute(element, Atom::OPEN).is_some(),
        PseudoClass::Required => document.attribute(element, Atom::REQUIRED).is_some(),
        PseudoClass::Optional => {
            matches!(html_name, Some(Atom::INPUT | Atom::SELECT | Atom::TEXTAREA))
                && document.attribute(element, Atom::REQUIRED).is_none()
        }
        PseudoClass::ReadWrite => {
            matches!(html_name, Some(Atom::INPUT | Atom::TEXTAREA))
                && document.attribute(element, Atom::READONLY).is_none()
        }
        PseudoClass::ReadOnly => {
            !(matches!(html_name, Some(Atom::INPUT | Atom::TEXTAREA))
                && document.attribute(element, Atom::READONLY).is_none())
        }
        _ => false,
    }
}

fn matches_nth(context: &Context<'_>, nth: &NthSelector, element: NodeId) -> bool {
    let document = context.document;
    let name = document.element_name(element);
    let counts = |node: NodeId| match (&nth.kind, &nth.of) {
        (NthKind::OfType | NthKind::LastOfType, _) => document.element_name(node) == name,
        (_, Some(of)) => matches_list(context, of, node),
        (_, None) => true,
    };
    if nth
        .of
        .as_ref()
        .is_some_and(|of| !matches_list(context, of, element))
    {
        return false;
    }
    let index = match nth.kind {
        NthKind::Child | NthKind::OfType => {
            std::iter::successors(previous_element(document, element), |&node| {
                previous_element(document, node)
            })
            .filter(|&node| counts(node))
            .count()
        }
        NthKind::LastChild | NthKind::LastOfType => {
            std::iter::successors(next_element(document, element), |&node| {
                next_element(document, node)
            })
            .filter(|&node| counts(node))
            .count()
        }
    };
    nth.matches_index(i32::try_from(index + 1).unwrap_or(i32::MAX))
}

pub(crate) struct Bloom {
    counters: Box<[u8; BLOOM_SIZE]>,
}

const BLOOM_SIZE: usize = 4096;

impl Bloom {
    pub(crate) fn new() -> Self {
        Bloom {
            counters: Box::new([0; BLOOM_SIZE]),
        }
    }

    fn slots(atom: Atom) -> [usize; 2] {
        let hash = atom.index().wrapping_mul(0x9E37_79B9);
        [
            (hash >> 20) as usize % BLOOM_SIZE,
            (hash & 0xFFF) as usize % BLOOM_SIZE,
        ]
    }

    fn atoms(document: &Document, element: NodeId) -> impl Iterator<Item = Atom> + '_ {
        let name = document.element_name(element).map(|(_, local)| local);
        name.into_iter()
            .chain(document.element_id(element))
            .chain(document.classes(element).iter().copied())
    }

    pub(crate) fn push(&mut self, document: &Document, element: NodeId) {
        for atom in Self::atoms(document, element) {
            for slot in Self::slots(atom) {
                if let Some(counter) = self.counters.get_mut(slot) {
                    *counter = counter.saturating_add(1);
                }
            }
        }
    }

    pub(crate) fn pop(&mut self, document: &Document, element: NodeId) {
        for atom in Self::atoms(document, element) {
            for slot in Self::slots(atom) {
                if let Some(counter) = self.counters.get_mut(slot)
                    && *counter != u8::MAX
                {
                    *counter = counter.saturating_sub(1);
                }
            }
        }
    }

    pub(crate) fn may_match(&self, selector: &Selector) -> bool {
        selector.ancestor_atoms.iter().flatten().all(|&atom| {
            Self::slots(atom)
                .iter()
                .all(|&slot| self.counters.get(slot).is_some_and(|&count| count > 0))
        })
    }
}
