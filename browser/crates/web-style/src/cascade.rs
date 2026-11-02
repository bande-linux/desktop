use std::collections::HashMap;
use std::sync::Arc;

use web_atoms::{Atom, AtomTable};
use web_css::properties::{
    CssWideKeyword, CustomValue, DeclaredValue, LonghandId, PropertyId, UnparsedValue,
    parse_value_str,
};
use web_css::stylesheet::parse_style_attribute;
use web_dom::{Document, NodeId, NodeKind};

use crate::compute::{Context, Value, compute};
use crate::matching::{self, Bloom};
use crate::stylist::{Key, Origin, Stylist, UNLAYERED};
use crate::{ComputedStyle, Display, InheritedStyle, ResetStyle};

const UNSTYLED: u32 = u32::MAX;
const SHARING_CANDIDATES: usize = 8;
const VAR_DEPTH: u8 = 16;

pub struct Restyle {
    pub changed: Vec<NodeId>,
    pub layout: bool,
}

pub struct Styles {
    rule_sets: HashMap<(u32, Box<[u32]>), u32>,
    per_node: Vec<u32>,
    pool: Vec<ComputedStyle>,
    initial_reset: Arc<ResetStyle>,
    restyled: usize,
}

struct Sibling {
    element: NodeId,
    style: u32,
}

struct Traversal<'a> {
    document: &'a Document,
    stylist: &'a Stylist,
    bloom: Bloom,
    root_font_size: f32,
}

type Candidate<'a> = (LonghandId, (u32, u32, u32), Origin, &'a DeclaredValue);

fn rank(origin: Origin, important: bool, layer: u16) -> u32 {
    let layer = u32::from(layer);
    let reversed = u32::from(UNLAYERED) - layer;
    match (origin, important) {
        (Origin::UserAgent, false) => layer,
        (Origin::Author, false) => 0x1_0000 + layer,
        (Origin::Author, true) => 0x2_0000 + reversed,
        (Origin::UserAgent, true) => 0x3_0000 + reversed,
    }
}

impl Styles {
    pub fn compute(document: &Document, stylist: &Stylist) -> Self {
        let root = ComputedStyle {
            inherited: Arc::new(InheritedStyle::initial()),
            reset: Arc::new(ResetStyle::initial()),
        };
        let initial_reset = root.reset.clone();
        let mut styles = Styles {
            rule_sets: HashMap::new(),
            per_node: vec![UNSTYLED; document.len()],
            pool: vec![root],
            initial_reset,
            restyled: 0,
        };
        let mut traversal = Traversal {
            document,
            stylist,
            bloom: Bloom::new(),
            root_font_size: crate::MEDIUM,
        };
        enum Step {
            Enter(NodeId, u32),
            Exit(NodeId),
        }
        let mut stack: Vec<Step> = document
            .children(NodeId::DOCUMENT)
            .map(|child| Step::Enter(child, 0))
            .collect();
        stack.reverse();
        let mut siblings: Vec<(NodeId, Vec<Sibling>)> = Vec::new();
        while let Some(step) = stack.pop() {
            let (node, parent) = match step {
                Step::Enter(node, parent) => (node, parent),
                Step::Exit(node) => {
                    traversal.bloom.pop(document, node);
                    continue;
                }
            };
            let style = match document.kind(node) {
                Some(NodeKind::Element) => {
                    let parent_node = document.parent(node).unwrap_or(NodeId::DOCUMENT);
                    let cache = match siblings.iter().position(|(owner, _)| *owner == parent_node) {
                        Some(index) => index,
                        None => {
                            siblings.push((parent_node, Vec::new()));
                            siblings.len() - 1
                        }
                    };
                    let shared = siblings.get(cache).and_then(|(_, cached)| {
                        cached
                            .iter()
                            .find(|sibling| traversal.can_share(node, sibling.element))
                            .map(|sibling| sibling.style)
                    });
                    let index = match shared {
                        Some(index) => index,
                        None => {
                            let index = styles.style_element(&traversal, node, parent);
                            if let Some((_, cached)) = siblings.get_mut(cache) {
                                if cached.len() >= SHARING_CANDIDATES {
                                    cached.remove(0);
                                }
                                cached.push(Sibling {
                                    element: node,
                                    style: index,
                                });
                            }
                            index
                        }
                    };
                    if document.parent(node) == Some(NodeId::DOCUMENT)
                        && let Some(style) = styles.pool.get(index as usize)
                    {
                        traversal.root_font_size = style.inherited.font_size;
                    }
                    if siblings.len() > 64 {
                        siblings.retain(|(owner, _)| document.ancestors(node).any(|a| a == *owner));
                    }
                    index
                }
                Some(NodeKind::Text) => parent,
                _ => continue,
            };
            if styles
                .pool
                .get(style as usize)
                .is_none_or(|style| style.display() == Display::None)
            {
                continue;
            }
            if let Some(slot) = styles.per_node.get_mut(node.index()) {
                *slot = style;
            }
            if document.kind(node) == Some(NodeKind::Element) {
                traversal.bloom.push(document, node);
                stack.push(Step::Exit(node));
            }
            let start = stack.len();
            stack.extend(
                document
                    .children(node)
                    .map(|child| Step::Enter(child, style)),
            );
            if let Some(added) = stack.get_mut(start..) {
                added.reverse();
            }
        }
        styles
    }

    pub fn get(&self, node: NodeId) -> Option<&ComputedStyle> {
        let index = *self.per_node.get(node.index())?;
        self.pool.get(index as usize)
    }

    pub fn anonymous(&self, parent: &ComputedStyle) -> ComputedStyle {
        ComputedStyle {
            inherited: parent.inherited.clone(),
            reset: self.initial_reset.clone(),
        }
    }

    pub fn distinct(&self) -> usize {
        self.pool.len()
    }

    pub fn restyled(&self) -> usize {
        self.restyled
    }

    pub fn heap_bytes(&self) -> usize {
        let groups: usize = self
            .pool
            .iter()
            .map(|style| {
                size_of::<InheritedStyle>() / Arc::strong_count(&style.inherited)
                    + size_of::<ResetStyle>() / Arc::strong_count(&style.reset)
            })
            .sum();
        let rule_sets: usize = self
            .rule_sets
            .keys()
            .map(|(_, matched)| matched.len() * size_of::<u32>() + 3 * size_of::<u64>())
            .sum();
        rule_sets
            + self.per_node.capacity() * size_of::<u32>()
            + self.pool.capacity() * size_of::<ComputedStyle>()
            + groups
    }

    pub fn restyle_hover(
        &mut self,
        document: &Document,
        stylist: &Stylist,
        changed: &[NodeId],
    ) -> Restyle {
        let matches_key = |element: NodeId, key: &Key| match key {
            Key::Any => true,
            Key::Id(id) => document.element_id(element) == Some(*id),
            Key::Class(class) => document.classes(element).contains(class),
            Key::Name(name) => {
                document.element_name(element).map(|(_, local)| local) == Some(*name)
            }
        };
        let mut dirty: Vec<NodeId> = Vec::new();
        for &element in changed {
            if stylist
                .hover_subjects
                .iter()
                .any(|key| matches_key(element, key))
            {
                dirty.push(element);
            }
            if stylist
                .hover_ancestors
                .iter()
                .any(|key| matches_key(element, key))
            {
                dirty.extend(element_descendants(document, element));
            }
        }
        dirty.sort_by_key(|node| document.ancestors(*node).count());
        dirty.dedup();
        self.restyled = 0;
        let mut updated = Vec::new();
        let mut layout = false;
        let mut index = 0;
        while let Some(&element) = dirty.get(index) {
            index += 1;
            let Some(parent_index) = document
                .parent(element)
                .and_then(|parent| self.per_node.get(parent.index()).copied())
                .filter(|&parent| parent != UNSTYLED)
                .or_else(|| (document.parent(element) == Some(NodeId::DOCUMENT)).then_some(0))
            else {
                continue;
            };
            let mut traversal = Traversal {
                document,
                stylist,
                bloom: Bloom::new(),
                root_font_size: self.root_font_size(document),
            };
            for ancestor in document.ancestors(element).skip(1) {
                if document.kind(ancestor) == Some(NodeKind::Element) {
                    traversal.bloom.push(document, ancestor);
                }
            }
            self.restyled += 1;
            let new = self.style_element(&traversal, element, parent_index);
            let old = self
                .per_node
                .get(element.index())
                .copied()
                .unwrap_or(UNSTYLED);
            let (Some(new_style), old_style) =
                (self.pool.get(new as usize), self.pool.get(old as usize))
            else {
                continue;
            };
            if old_style == Some(new_style) {
                continue;
            }
            let inherited_changed =
                old_style.is_none_or(|old| old.inherited != new_style.inherited);
            layout |= old_style.is_none_or(|old| !old.same_layout(new_style));
            if let Some(slot) = self.per_node.get_mut(element.index()) {
                *slot = new;
            }
            for child in document.children(element) {
                if document.kind(child) == Some(NodeKind::Text)
                    && let Some(slot) = self.per_node.get_mut(child.index())
                {
                    *slot = new;
                }
            }
            updated.push(element);
            if inherited_changed {
                for child in document.children(element) {
                    if document.kind(child) == Some(NodeKind::Element) && !dirty.contains(&child) {
                        dirty.push(child);
                    }
                }
            }
        }
        Restyle {
            changed: updated,
            layout,
        }
    }

    fn root_font_size(&self, document: &Document) -> f32 {
        document
            .children(NodeId::DOCUMENT)
            .find_map(|child| self.get(child))
            .map_or(crate::MEDIUM, |style| style.inherited.font_size)
    }

    fn style_element(&mut self, traversal: &Traversal<'_>, element: NodeId, parent: u32) -> u32 {
        let Some(parent_style) = self.pool.get(parent as usize).cloned() else {
            return 0;
        };
        let matched = traversal.matched(element);
        let cacheable = traversal.document.attribute(element, Atom::STYLE).is_none()
            && traversal.document.parent(element) != Some(NodeId::DOCUMENT);
        let key = (parent, matched.into_boxed_slice());
        if cacheable && let Some(&index) = self.rule_sets.get(&key) {
            return index;
        }
        let style = traversal.style(element, &key.1, &parent_style, &self.initial_reset);
        let index = self.intern(style, parent, &parent_style);
        if cacheable {
            self.rule_sets.insert(key, index);
        }
        index
    }

    fn intern(&mut self, style: ComputedStyle, parent: u32, parent_style: &ComputedStyle) -> u32 {
        if Arc::ptr_eq(&style.inherited, &parent_style.inherited)
            && Arc::ptr_eq(&style.reset, &parent_style.reset)
        {
            return parent;
        }
        self.pool.push(style);
        (self.pool.len() - 1) as u32
    }
}

fn element_descendants(document: &Document, root: NodeId) -> Vec<NodeId> {
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if document.kind(node) == Some(NodeKind::Element) {
            found.push(node);
            stack.extend(document.children(node));
        }
    }
    found
}

impl Traversal<'_> {
    fn candidates(&self, element: NodeId) -> impl Iterator<Item = u32> + '_ {
        let document = self.document;
        let name = document
            .element_name(element)
            .map_or(Atom::HTML, |(_, local)| local);
        self.stylist.candidates(
            document.element_id(element),
            document.classes(element),
            name,
        )
    }

    fn context(&self) -> matching::Context<'_> {
        matching::Context {
            document: self.document,
            namespaces: &self.stylist.namespaces,
            anchor: None,
        }
    }

    fn can_share(&self, element: NodeId, sibling: NodeId) -> bool {
        let document = self.document;
        let plain = |node: NodeId| {
            document.element_id(node).is_none()
                && document.attribute(node, Atom::STYLE).is_none()
                && !document.is_hovered(node)
        };
        if !plain(element)
            || !plain(sibling)
            || document.element_name(element) != document.element_name(sibling)
            || document.classes(element) != document.classes(sibling)
        {
            return false;
        }
        let context = self.context();
        self.candidates(element).all(|entry| {
            let Some(entry) = self.stylist.entries.get(entry as usize) else {
                return true;
            };
            !entry.sensitive
                || entry.selector().is_none_or(|selector| {
                    matching::matches(&context, selector, element)
                        == matching::matches(&context, selector, sibling)
                })
        })
    }

    fn matched(&self, element: NodeId) -> Vec<u32> {
        let context = self.context();
        let mut matched: Vec<u32> = self
            .candidates(element)
            .filter(|&entry| {
                self.stylist
                    .entries
                    .get(entry as usize)
                    .and_then(|entry| entry.selector())
                    .is_some_and(|selector| {
                        self.bloom.may_match(selector)
                            && matching::matches(&context, selector, element)
                    })
            })
            .collect();
        matched.sort_unstable();
        matched.dedup();
        matched
    }

    fn style(
        &self,
        element: NodeId,
        matched: &[u32],
        parent: &ComputedStyle,
        initial_reset: &Arc<ResetStyle>,
    ) -> ComputedStyle {
        let mut atoms = AtomTable::new();
        let style_attribute = self
            .document
            .attribute(element, Atom::STYLE)
            .map(|css| parse_style_attribute(css, &mut atoms));
        let names = self.document.atoms();
        let mut candidates: Vec<Candidate<'_>> = Vec::new();
        let mut customs: Vec<(&str, (u32, u32, u32), &CustomValue)> = Vec::new();
        for &index in matched {
            let Some(entry) = self.stylist.entries.get(index as usize) else {
                continue;
            };
            let Some(rule) = self.stylist.rules.get(entry.rule as usize) else {
                continue;
            };
            let specificity = entry.selector().map_or(0, |selector| selector.specificity);
            for declaration in &rule.block.declarations {
                let key = (
                    rank(rule.origin, declaration.important, rule.layer),
                    specificity,
                    entry.rule,
                );
                candidates.push((declaration.id, key, rule.origin, &declaration.value));
            }
            for custom in &rule.block.custom {
                let key = (
                    rank(rule.origin, custom.important, rule.layer),
                    specificity,
                    entry.rule,
                );
                customs.push((names.name(custom.name), key, &custom.value));
            }
        }
        if let Some(block) = &style_attribute {
            for declaration in &block.declarations {
                let key = (
                    rank(Origin::Author, declaration.important, UNLAYERED),
                    u32::MAX,
                    u32::MAX,
                );
                candidates.push((declaration.id, key, Origin::Author, &declaration.value));
            }
            for custom in &block.custom {
                let key = (
                    rank(Origin::Author, custom.important, UNLAYERED),
                    u32::MAX,
                    u32::MAX,
                );
                customs.push((atoms.name(custom.name), key, &custom.value));
            }
        }
        let custom = cascade_custom(parent, customs);
        let winners = select_winners(candidates);
        let substituted: Vec<Option<DeclaredValue>> = winners
            .iter()
            .map(|(id, value)| match value {
                DeclaredValue::Unparsed(unparsed) => Some(
                    substitute_unparsed(*id, unparsed, &custom)
                        .unwrap_or(DeclaredValue::CssWide(CssWideKeyword::Unset)),
                ),
                _ => None,
            })
            .collect();
        let values: Vec<(LonghandId, Value<'_>)> = winners
            .iter()
            .zip(&substituted)
            .filter_map(|((id, declared), substitute)| {
                match substitute.as_ref().unwrap_or(declared) {
                    DeclaredValue::Value(value) => Some((*id, Value::Declared(value))),
                    DeclaredValue::CssWide(keyword) => Some((*id, Value::Keyword(*keyword))),
                    DeclaredValue::Unparsed(_) => None,
                }
            })
            .collect();
        let context = Context {
            root_font_size: self.root_font_size,
            viewport: self.stylist.viewport,
            root: self.document.parent(element) == Some(NodeId::DOCUMENT),
        };
        compute(parent, initial_reset, values, custom, &context)
    }
}

fn select_winners(mut candidates: Vec<Candidate<'_>>) -> Vec<(LonghandId, &DeclaredValue)> {
    candidates.sort_by_key(|candidate| (candidate.0, candidate.1));
    let mut winners = Vec::new();
    for group in candidates.chunk_by(|a, b| a.0 == b.0) {
        let mut end = group.len();
        while let Some(&(id, key, origin, value)) =
            end.checked_sub(1).and_then(|last| group.get(last))
        {
            match value {
                DeclaredValue::CssWide(CssWideKeyword::Revert) => {
                    end = group
                        .get(..end)
                        .unwrap_or_default()
                        .iter()
                        .take_while(|candidate| candidate.2 < origin)
                        .count();
                }
                DeclaredValue::CssWide(CssWideKeyword::RevertLayer) => {
                    end = group
                        .get(..end)
                        .unwrap_or_default()
                        .iter()
                        .take_while(|candidate| candidate.1.0 < key.0)
                        .count();
                }
                _ => {
                    winners.push((id, value));
                    break;
                }
            }
        }
    }
    winners
}

type CustomMap = Arc<[(Arc<str>, Arc<str>)]>;

fn cascade_custom(
    parent: &ComputedStyle,
    mut customs: Vec<(&str, (u32, u32, u32), &CustomValue)>,
) -> CustomMap {
    if customs.is_empty() {
        return parent.inherited.custom.clone();
    }
    customs.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    let mut map: Vec<(Arc<str>, Arc<str>)> = parent.inherited.custom.to_vec();
    for group in customs.chunk_by(|a, b| a.0 == b.0) {
        let Some(&(name, _, value)) = group.last() else {
            continue;
        };
        let existing = map.iter().position(|(known, _)| &**known == name);
        match value {
            CustomValue::Value(value) => match existing.and_then(|index| map.get_mut(index)) {
                Some(slot) => slot.1 = value.clone(),
                None => map.push((name.into(), value.clone())),
            },
            CustomValue::CssWide(CssWideKeyword::Initial) => {
                if let Some(index) = existing {
                    map.remove(index);
                }
            }
            CustomValue::CssWide(_) => {}
        }
    }
    map.into()
}

const SUBSTITUTION_LIMIT: usize = 1 << 20;

fn substitute_unparsed(
    id: LonghandId,
    unparsed: &UnparsedValue,
    custom: &CustomMap,
) -> Option<DeclaredValue> {
    let css = substitute_vars(&unparsed.css, custom, 0)?;
    let property = unparsed
        .shorthand
        .map_or(PropertyId::Longhand(id), PropertyId::Shorthand);
    parse_value_str(property, &css)
        .ok()?
        .into_iter()
        .find(|(longhand, _)| *longhand == id)
        .map(|(_, value)| value)
}

fn substitute_vars(css: &str, custom: &CustomMap, depth: u8) -> Option<String> {
    if depth > VAR_DEPTH {
        return None;
    }
    let mut output = String::with_capacity(css.len());
    let mut rest = css;
    loop {
        let lower = rest.to_ascii_lowercase();
        let found = ["var(", "env("]
            .iter()
            .filter_map(|function| lower.find(function))
            .min();
        let Some(start) = found else {
            output.push_str(rest);
            break;
        };
        output.push_str(rest.get(..start)?);
        let inner_start = start + 4;
        let mut nesting = 0usize;
        let mut end = None;
        for (offset, ch) in rest.get(inner_start..)?.char_indices() {
            match ch {
                '(' => nesting += 1,
                ')' if nesting == 0 => {
                    end = Some(inner_start + offset);
                    break;
                }
                ')' => nesting -= 1,
                _ => {}
            }
        }
        let end = end.unwrap_or(rest.len());
        let inner = rest.get(inner_start..end)?;
        let is_env = lower.get(start..start + 4) == Some("env(");
        let (name, fallback) = split_fallback(inner);
        let value = (!is_env)
            .then(|| {
                custom
                    .iter()
                    .find(|(known, _)| &**known == name.trim())
                    .map(|(_, value)| value.clone())
            })
            .flatten();
        let replacement = match (value, fallback) {
            (Some(value), _) => substitute_vars(&value, custom, depth + 1)?,
            (None, Some(fallback)) => substitute_vars(fallback, custom, depth + 1)?,
            (None, None) => return None,
        };
        output.push_str(&replacement);
        if output.len() > SUBSTITUTION_LIMIT {
            return None;
        }
        rest = rest.get((end + 1).min(rest.len())..)?;
    }
    Some(output)
}

fn split_fallback(inner: &str) -> (&str, Option<&str>) {
    let mut nesting = 0usize;
    for (offset, ch) in inner.char_indices() {
        match ch {
            '(' => nesting += 1,
            ')' => nesting = nesting.saturating_sub(1),
            ',' if nesting == 0 => {
                return (
                    inner.get(..offset).unwrap_or_default(),
                    inner.get(offset + 1..),
                );
            }
            _ => {}
        }
    }
    (inner, None)
}
