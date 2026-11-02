use std::num::NonZeroU32;

use web_atoms::{Atom, AtomTable};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct NodeId(NonZeroU32);

impl NodeId {
    pub const DOCUMENT: NodeId = NodeId(NonZeroU32::MIN);

    pub fn index(self) -> usize {
        self.0.get() as usize - 1
    }

    fn from_index(index: usize) -> Option<NodeId> {
        u32::try_from(index + 1)
            .ok()
            .and_then(NonZeroU32::new)
            .map(NodeId)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum NodeKind {
    Document,
    Doctype,
    Element,
    Text,
    Comment,
    Fragment,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
pub enum Namespace {
    #[default]
    Html,
    Svg,
    MathMl,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum QuirksMode {
    #[default]
    NoQuirks,
    LimitedQuirks,
    Quirks,
}

#[derive(Clone, Copy)]
struct Span {
    start: u32,
    len: u32,
}

#[derive(Clone, Copy)]
struct Node {
    parent: Option<NodeId>,
    first_child: Option<NodeId>,
    last_child: Option<NodeId>,
    previous_sibling: Option<NodeId>,
    next_sibling: Option<NodeId>,
    kind: NodeKind,
    namespace: Namespace,
    name: Atom,
    data: Span,
}

#[derive(Clone, Copy)]
struct AttributeSlot {
    name: Atom,
    value: Span,
}

pub struct Document {
    nodes: Vec<Node>,
    attributes: Vec<AttributeSlot>,
    attribute_text: String,
    text: String,
    atoms: AtomTable,
    templates: Vec<(NodeId, NodeId)>,
    quirks_mode: QuirksMode,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

pub const NODE_CORE_BYTES: usize = size_of::<Node>();

impl Document {
    pub fn new() -> Self {
        let mut document = Self {
            nodes: Vec::new(),
            attributes: Vec::new(),
            attribute_text: String::new(),
            text: String::new(),
            atoms: AtomTable::new(),
            templates: Vec::new(),
            quirks_mode: QuirksMode::NoQuirks,
        };
        document.push(NodeKind::Document, Namespace::Html, Atom::HTML, EMPTY);
        document
    }

    fn push(&mut self, kind: NodeKind, namespace: Namespace, name: Atom, data: Span) -> NodeId {
        let id = NodeId::from_index(self.nodes.len()).unwrap_or(NodeId::DOCUMENT);
        self.nodes.push(Node {
            parent: None,
            first_child: None,
            last_child: None,
            previous_sibling: None,
            next_sibling: None,
            kind,
            namespace,
            name,
            data,
        });
        id
    }

    fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.index())
    }

    fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.index())
    }

    pub fn atoms(&self) -> &AtomTable {
        &self.atoms
    }

    pub fn intern(&mut self, name: &str) -> Atom {
        self.atoms.intern(name)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }

    pub fn quirks_mode(&self) -> QuirksMode {
        self.quirks_mode
    }

    pub fn set_quirks_mode(&mut self, mode: QuirksMode) {
        self.quirks_mode = mode;
    }

    pub fn kind(&self, id: NodeId) -> Option<NodeKind> {
        self.node(id).map(|node| node.kind)
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id)?.parent
    }

    pub fn first_child(&self, id: NodeId) -> Option<NodeId> {
        self.node(id)?.first_child
    }

    pub fn last_child(&self, id: NodeId) -> Option<NodeId> {
        self.node(id)?.last_child
    }

    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.node(id)?.next_sibling
    }

    pub fn previous_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.node(id)?.previous_sibling
    }

    pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(self.first_child(id), |&child| self.next_sibling(child))
    }

    pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(Some(id), |&node| self.parent(node))
    }

    pub fn element_name(&self, id: NodeId) -> Option<(Namespace, Atom)> {
        self.node(id)
            .filter(|node| node.kind == NodeKind::Element)
            .map(|node| (node.namespace, node.name))
    }

    pub fn html_element_name(&self, id: NodeId) -> Option<Atom> {
        match self.element_name(id) {
            Some((Namespace::Html, name)) => Some(name),
            _ => None,
        }
    }

    pub fn doctype_name(&self, id: NodeId) -> Option<&str> {
        self.node(id)
            .filter(|node| node.kind == NodeKind::Doctype)
            .map(|node| self.atoms.name(node.name))
    }

    pub fn text(&self, id: NodeId) -> Option<&str> {
        self.node(id)
            .filter(|node| matches!(node.kind, NodeKind::Text | NodeKind::Comment))
            .map(|node| span_text(&self.text, node.data))
    }

    pub fn attributes(&self, id: NodeId) -> impl Iterator<Item = (Atom, &str)> + '_ {
        let slots = self
            .node(id)
            .filter(|node| node.kind == NodeKind::Element)
            .and_then(|node| {
                self.attributes
                    .get(node.data.start as usize..(node.data.start + node.data.len) as usize)
            })
            .unwrap_or_default();
        slots
            .iter()
            .map(|slot| (slot.name, span_text(&self.attribute_text, slot.value)))
    }

    pub fn attribute(&self, id: NodeId, name: Atom) -> Option<&str> {
        self.attributes(id)
            .find(|&(attribute, _)| attribute == name)
            .map(|(_, value)| value)
    }

    pub fn template_contents(&self, template: NodeId) -> Option<NodeId> {
        self.templates
            .iter()
            .find(|&&(owner, _)| owner == template)
            .map(|&(_, contents)| contents)
    }

    pub fn create_element<'a>(
        &mut self,
        namespace: Namespace,
        name: Atom,
        attributes: impl IntoIterator<Item = (Atom, &'a str)>,
    ) -> NodeId {
        let start = self.attributes.len() as u32;
        for (attribute, value) in attributes {
            let value = push_span(&mut self.attribute_text, value);
            self.attributes.push(AttributeSlot {
                name: attribute,
                value,
            });
        }
        let len = self.attributes.len() as u32 - start;
        let id = self.push(NodeKind::Element, namespace, name, Span { start, len });
        if namespace == Namespace::Html && name == Atom::TEMPLATE {
            let contents = self.push(NodeKind::Fragment, Namespace::Html, name, EMPTY);
            self.templates.push((id, contents));
        }
        id
    }

    pub fn add_attributes_if_missing<'a>(
        &mut self,
        element: NodeId,
        attributes: impl IntoIterator<Item = (Atom, &'a str)>,
    ) {
        for (name, value) in attributes {
            if self.attribute(element, name).is_some() {
                continue;
            }
            let Some(node) = self.node(element).copied() else {
                return;
            };
            let (start, len) = (node.data.start as usize, node.data.len as usize);
            if start + len != self.attributes.len() {
                let existing = self.attributes.get(start..start + len).unwrap_or_default();
                let moved = existing.to_vec();
                self.attributes.extend(moved);
            }
            let value = push_span(&mut self.attribute_text, value);
            self.attributes.push(AttributeSlot { name, value });
            let new_start = (self.attributes.len() - len - 1) as u32;
            if let Some(node) = self.node_mut(element) {
                node.data = Span {
                    start: new_start,
                    len: len as u32 + 1,
                };
            }
        }
    }

    pub fn create_text(&mut self, text: &str) -> NodeId {
        let data = push_span(&mut self.text, text);
        self.push(NodeKind::Text, Namespace::Html, Atom::HTML, data)
    }

    pub fn create_comment(&mut self, text: &str) -> NodeId {
        let data = push_span(&mut self.text, text);
        self.push(NodeKind::Comment, Namespace::Html, Atom::HTML, data)
    }

    pub fn create_doctype(&mut self, name: &str) -> NodeId {
        let name = self.atoms.intern(name);
        self.push(NodeKind::Doctype, Namespace::Html, name, EMPTY)
    }

    pub fn append(&mut self, parent: NodeId, child: NodeId) {
        self.insert(parent, child, None);
    }

    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: Option<NodeId>) {
        self.insert(parent, child, reference);
    }

    fn insert(&mut self, parent: NodeId, child: NodeId, reference: Option<NodeId>) {
        if parent == child
            || self.node(parent).is_none()
            || self.node(child).is_none()
            || reference.is_some_and(|reference| self.parent(reference) != Some(parent))
            || self.ancestors(parent).any(|ancestor| ancestor == child)
        {
            return;
        }
        self.detach(child);
        let previous = match reference {
            Some(reference) => self.previous_sibling(reference),
            None => self.last_child(parent),
        };
        if let Some(node) = self.node_mut(child) {
            node.parent = Some(parent);
            node.previous_sibling = previous;
            node.next_sibling = reference;
        }
        match previous.and_then(|previous| self.node_mut(previous)) {
            Some(previous) => previous.next_sibling = Some(child),
            None => {
                if let Some(parent) = self.node_mut(parent) {
                    parent.first_child = Some(child);
                }
            }
        }
        match reference.and_then(|reference| self.node_mut(reference)) {
            Some(reference) => reference.previous_sibling = Some(child),
            None => {
                if let Some(parent) = self.node_mut(parent) {
                    parent.last_child = Some(child);
                }
            }
        }
    }

    pub fn detach(&mut self, child: NodeId) {
        let Some(node) = self.node(child).copied() else {
            return;
        };
        let Some(parent) = node.parent else {
            return;
        };
        match node
            .previous_sibling
            .and_then(|previous| self.node_mut(previous))
        {
            Some(previous) => previous.next_sibling = node.next_sibling,
            None => {
                if let Some(parent) = self.node_mut(parent) {
                    parent.first_child = node.next_sibling;
                }
            }
        }
        match node.next_sibling.and_then(|next| self.node_mut(next)) {
            Some(next) => next.previous_sibling = node.previous_sibling,
            None => {
                if let Some(parent) = self.node_mut(parent) {
                    parent.last_child = node.previous_sibling;
                }
            }
        }
        if let Some(node) = self.node_mut(child) {
            node.parent = None;
            node.previous_sibling = None;
            node.next_sibling = None;
        }
    }

    pub fn reparent_children(&mut self, from: NodeId, to: NodeId) {
        while let Some(child) = self.first_child(from) {
            let before = self.first_child(from);
            self.append(to, child);
            if self.first_child(from) == before {
                self.detach(child);
            }
        }
    }

    pub fn append_text(&mut self, parent: NodeId, text: &str) {
        match self.last_child(parent) {
            Some(last) if self.kind(last) == Some(NodeKind::Text) => self.extend_text(last, text),
            _ => {
                let node = self.create_text(text);
                self.append(parent, node);
            }
        }
    }

    pub fn insert_text_before(&mut self, parent: NodeId, reference: NodeId, text: &str) {
        match self.previous_sibling(reference) {
            Some(previous) if self.kind(previous) == Some(NodeKind::Text) => {
                self.extend_text(previous, text)
            }
            _ => {
                let node = self.create_text(text);
                self.insert_before(parent, node, Some(reference));
            }
        }
    }

    fn extend_text(&mut self, id: NodeId, text: &str) {
        let Some(node) = self.node(id).copied() else {
            return;
        };
        let end = (node.data.start + node.data.len) as usize;
        let data = if end == self.text.len() {
            self.text.push_str(text);
            Span {
                start: node.data.start,
                len: node.data.len + text.len() as u32,
            }
        } else {
            let start = self.text.len() as u32;
            self.text.extend_from_within(node.data.start as usize..end);
            self.text.push_str(text);
            Span {
                start,
                len: node.data.len + text.len() as u32,
            }
        };
        if let Some(node) = self.node_mut(id) {
            node.data = data;
        }
    }

    pub fn elements(&self) -> usize {
        self.nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Element)
            .count()
    }

    pub fn heap_bytes(&self) -> usize {
        self.nodes.capacity() * size_of::<Node>()
            + self.attributes.capacity() * size_of::<AttributeSlot>()
            + self.attribute_text.capacity()
            + self.text.capacity()
            + self.templates.capacity() * size_of::<(NodeId, NodeId)>()
            + self.atoms.heap_bytes()
    }
}

const EMPTY: Span = Span { start: 0, len: 0 };

fn push_span(buffer: &mut String, text: &str) -> Span {
    let start = buffer.len() as u32;
    buffer.push_str(text);
    Span {
        start,
        len: text.len() as u32,
    }
}

fn span_text(buffer: &str, span: Span) -> &str {
    buffer
        .get(span.start as usize..(span.start + span.len) as usize)
        .unwrap_or_default()
}
