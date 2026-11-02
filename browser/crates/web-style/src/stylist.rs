use std::collections::HashMap;

use web_atoms::{Atom, AtomTable, Namespace};
use web_css::font_face::FontFace;
use web_css::media::{MediaEnvironment, MediaList};
use web_css::properties::DeclarationBlock;
use web_css::selectors::{
    Combinator, Component, PseudoClass, PseudoElement, Selector, SelectorList,
};
use web_css::stylesheet::{CssRule, LayerName, Stylesheet, parse_stylesheet};

const UA_SHEET: &str = include_str!("ua.css");

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Origin {
    UserAgent,
    Author,
}

pub(crate) const UNLAYERED: u16 = u16::MAX;

pub(crate) struct RuleData {
    pub(crate) block: DeclarationBlock,
    pub(crate) origin: Origin,
    pub(crate) layer: u16,
    pub(crate) order: u32,
    media: Box<[u32]>,
}

pub(crate) struct Entry {
    list: SelectorList,
    index: u32,
    pub(crate) rule: u32,
    pub(crate) sensitive: bool,
}

impl Entry {
    pub(crate) fn selector(&self) -> Option<&Selector> {
        self.list.0.get(self.index as usize)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Key {
    Id(Atom),
    Class(Atom),
    Name(Atom),
    Any,
}

pub struct Stylist {
    pub(crate) rules: Vec<RuleData>,
    pub(crate) entries: Vec<Entry>,
    buckets: HashMap<Key, (u32, u32)>,
    bucketed: Vec<u32>,
    media: Vec<MediaList>,
    media_matches: Vec<bool>,
    pub(crate) namespaces: Vec<(Atom, Namespace)>,
    pub(crate) hover_subjects: Vec<Key>,
    pub(crate) hover_ancestors: Vec<Key>,
    pub(crate) viewport: Viewport,
    pub(crate) pseudos: bool,
    layers: Vec<Box<[Atom]>>,
    font_faces: Vec<(FontFace, Box<[u32]>)>,
}

struct Builder<'a> {
    stylist: &'a mut Stylist,
    position: u32,
    next: u32,
}

const RULES_PER_SHEET: u32 = 1 << 20;

impl Stylist {
    pub fn new(atoms: &mut AtomTable, viewport: Viewport) -> Self {
        let namespaces = [
            ("http://www.w3.org/1999/xhtml", Namespace::Html),
            ("http://www.w3.org/2000/svg", Namespace::Svg),
            ("http://www.w3.org/1998/Math/MathML", Namespace::MathMl),
        ]
        .into_iter()
        .map(|(url, namespace)| (atoms.intern(url), namespace))
        .collect();
        let mut stylist = Stylist {
            rules: Vec::new(),
            entries: Vec::new(),
            buckets: HashMap::new(),
            bucketed: Vec::new(),
            media: Vec::new(),
            media_matches: Vec::new(),
            namespaces,
            hover_subjects: Vec::new(),
            hover_ancestors: Vec::new(),
            viewport,
            pseudos: false,
            layers: Vec::new(),
            font_faces: Vec::new(),
        };
        let user_agent = parse_stylesheet(UA_SHEET, atoms);
        stylist.add(user_agent, Origin::UserAgent, 0);
        stylist
    }

    pub fn add_sheet(&mut self, sheet: Stylesheet, position: u32) {
        self.add(sheet, Origin::Author, position.saturating_add(1));
    }

    fn add(&mut self, sheet: Stylesheet, origin: Origin, position: u32) {
        let mut builder = Builder {
            stylist: self,
            position,
            next: 0,
        };
        builder.add_rules(sheet.rules, origin, UNLAYERED, &[]);
        self.reindex();
    }

    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport;
        let environment = environment(viewport);
        self.media_matches = self
            .media
            .iter()
            .map(|media| media.matches(&environment))
            .collect();
    }

    fn reindex(&mut self) {
        let mut keyed: Vec<(Key, u32)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                Some((subject_key(&entry.selector()?.components), index as u32))
            })
            .collect();
        keyed.sort_by_key(|(key, index)| (key_order(*key), *index));
        self.buckets.clear();
        self.bucketed.clear();
        self.bucketed.shrink_to(keyed.len());
        for group in keyed.chunk_by(|a, b| a.0 == b.0) {
            let start = self.bucketed.len() as u32;
            self.bucketed.extend(group.iter().map(|(_, index)| *index));
            if let Some((key, _)) = group.first() {
                self.buckets.insert(*key, (start, group.len() as u32));
            }
        }
        self.buckets.shrink_to_fit();
        let environment = environment(self.viewport);
        self.media_matches = self
            .media
            .iter()
            .map(|media| media.matches(&environment))
            .collect();
    }

    pub(crate) fn active(&self, rule: u32) -> bool {
        self.rules
            .get(rule as usize)
            .is_some_and(|rule| self.media_active(&rule.media))
    }

    fn media_active(&self, media: &[u32]) -> bool {
        media.iter().all(|&media| {
            self.media_matches
                .get(media as usize)
                .copied()
                .unwrap_or(false)
        })
    }

    pub fn font_faces(&self) -> impl Iterator<Item = &FontFace> {
        self.font_faces
            .iter()
            .filter(|(_, media)| self.media_active(media))
            .map(|(face, _)| face)
    }

    pub fn heap_bytes(&self) -> usize {
        let declarations: usize = self
            .rules
            .iter()
            .map(|rule| rule.block.declarations.capacity() * 64 + rule.block.custom.capacity() * 48)
            .sum();
        declarations
            + self.rules.capacity() * size_of::<RuleData>()
            + self.entries.capacity() * size_of::<Entry>()
            + self.bucketed.capacity() * size_of::<u32>()
            + self.buckets.capacity() * size_of::<(Key, (u32, u32))>()
    }

    pub(crate) fn candidates<'a>(
        &'a self,
        id: Option<Atom>,
        classes: &'a [Atom],
        name: Atom,
    ) -> impl Iterator<Item = u32> + 'a {
        let keys = id
            .map(Key::Id)
            .into_iter()
            .chain(classes.iter().map(|&class| Key::Class(class)))
            .chain([Key::Name(name), Key::Any]);
        keys.filter_map(|key| self.buckets.get(&key))
            .flat_map(|&(start, len)| {
                self.bucketed
                    .get(start as usize..(start + len) as usize)
                    .unwrap_or_default()
            })
            .copied()
            .filter(|&entry| {
                self.entries
                    .get(entry as usize)
                    .is_some_and(|entry| self.active(entry.rule))
            })
    }
}

fn key_order(key: Key) -> (u8, u32) {
    match key {
        Key::Id(atom) => (0, atom.index()),
        Key::Class(atom) => (1, atom.index()),
        Key::Name(atom) => (2, atom.index()),
        Key::Any => (3, 0),
    }
}

fn environment(viewport: Viewport) -> MediaEnvironment {
    MediaEnvironment {
        print: false,
        width: viewport.width,
        height: viewport.height,
        device_width: viewport.width,
        device_height: viewport.height,
        resolution: 1.0,
        font_size: crate::MEDIUM,
        dark: false,
        reduced_motion: false,
        hover: true,
        fine_pointer: true,
        wide_gamut: false,
    }
}

impl Builder<'_> {
    fn layer(&mut self, parent: Option<&[Atom]>, name: Option<&LayerName>) -> u16 {
        let layers = &mut self.stylist.layers;
        let index = match name {
            None => {
                layers.push(Box::new([]));
                layers.len() - 1
            }
            Some(name) => {
                let mut path: Vec<Atom> = parent.map(<[Atom]>::to_vec).unwrap_or_default();
                path.extend(name.0.iter().copied());
                match layers.iter().position(|layer| **layer == *path) {
                    Some(index) => index,
                    None => {
                        layers.push(path.into_boxed_slice());
                        layers.len() - 1
                    }
                }
            }
        };
        u16::try_from(index).map_or(UNLAYERED - 1, |index| index.min(UNLAYERED - 1))
    }

    fn layer_path(&self, layer: u16) -> Option<Box<[Atom]>> {
        (layer != UNLAYERED)
            .then(|| self.stylist.layers.get(usize::from(layer)).cloned())
            .flatten()
    }

    fn add_rules(&mut self, rules: Vec<CssRule>, origin: Origin, layer: u16, media: &[u32]) {
        for rule in rules {
            match rule {
                CssRule::Style(style) => {
                    let style = *style;
                    let index = self.stylist.rules.len() as u32;
                    let order = self
                        .position
                        .saturating_mul(RULES_PER_SHEET)
                        .saturating_add(self.next.min(RULES_PER_SHEET - 1));
                    self.next += 1;
                    let mut block = style.declarations;
                    block.declarations.shrink_to_fit();
                    block.custom.shrink_to_fit();
                    self.stylist.rules.push(RuleData {
                        block,
                        origin,
                        layer,
                        order,
                        media: media.into(),
                    });
                    for selector in 0..style.selectors.0.len() {
                        self.add_selector(&style.selectors, selector as u32, index);
                    }
                    self.add_rules(style.rules, origin, layer, media);
                }
                CssRule::Media(list, rules) => {
                    let index = self.stylist.media.len() as u32;
                    self.stylist.media.push(*list);
                    let nested: Vec<u32> = media.iter().copied().chain([index]).collect();
                    self.add_rules(rules, origin, layer, &nested);
                }
                CssRule::Supports(true, rules) | CssRule::Container(_, _, rules) => {
                    self.add_rules(rules, origin, layer, media)
                }
                CssRule::Layer(name, rules) => {
                    let parent = self.layer_path(layer);
                    let layer = self.layer(parent.as_deref(), name.as_ref());
                    self.add_rules(rules, origin, layer, media);
                }
                CssRule::FontFace(face) => {
                    if !face.family.is_empty() && !face.sources.is_empty() {
                        self.stylist.font_faces.push((*face, media.into()));
                    }
                }
                CssRule::LayerStatement(names) => {
                    let parent = self.layer_path(layer);
                    for name in names.iter() {
                        self.layer(parent.as_deref(), Some(name));
                    }
                }
                _ => {}
            }
        }
    }

    fn add_selector(&mut self, list: &SelectorList, index: u32, rule: u32) {
        let Some(selector) = list.0.get(index as usize) else {
            return;
        };
        for (subject, hover_key) in hover_keys(&selector.components) {
            let keys = if subject {
                &mut self.stylist.hover_subjects
            } else {
                &mut self.stylist.hover_ancestors
            };
            if !keys.contains(&hover_key) {
                keys.push(hover_key);
            }
        }
        if matches!(
            selector.pseudo_element,
            Some(PseudoElement::Before | PseudoElement::After)
        ) {
            self.stylist.pseudos = true;
        }
        self.stylist.entries.push(Entry {
            sensitive: is_sensitive(&selector.components),
            list: list.clone(),
            index,
            rule,
        });
    }
}

fn compound_key(compound: &[Component]) -> Key {
    let mut key = Key::Any;
    for component in compound {
        match component {
            Component::Id(id) => return Key::Id(*id),
            Component::Class(class) if !matches!(key, Key::Class(_)) => key = Key::Class(*class),
            Component::LocalName { lower, .. } if key == Key::Any => key = Key::Name(*lower),
            _ => {}
        }
    }
    key
}

fn subject_key(components: &[Component]) -> Key {
    let end = components
        .iter()
        .position(|component| matches!(component, Component::Combinator(_)))
        .unwrap_or(components.len());
    compound_key(components.get(..end).unwrap_or_default())
}

fn contains_hover(component: &Component) -> bool {
    match component {
        Component::PseudoClass(PseudoClass::Hover) => true,
        Component::Not(list)
        | Component::Is(list)
        | Component::Where(list)
        | Component::Has(list) => list_contains(list, &|component| contains_hover(component)),
        _ => false,
    }
}

fn list_contains(list: &SelectorList, test: &dyn Fn(&Component) -> bool) -> bool {
    list.0
        .iter()
        .any(|selector| selector.components.iter().any(test))
}

fn hover_keys(components: &[Component]) -> Vec<(bool, Key)> {
    components
        .split(|component| matches!(component, Component::Combinator(_)))
        .enumerate()
        .filter(|(_, compound)| compound.iter().any(contains_hover))
        .map(|(index, compound)| (index == 0, compound_key(compound)))
        .collect()
}

fn is_sensitive(components: &[Component]) -> bool {
    components.iter().any(|component| match component {
        Component::Combinator(Combinator::NextSibling | Combinator::LaterSibling)
        | Component::Attribute(_)
        | Component::Nth(_)
        | Component::Has(_)
        | Component::Lang(_)
        | Component::Dir(_) => true,
        Component::PseudoClass(pseudo) => !matches!(pseudo, PseudoClass::Root),
        Component::Not(list) | Component::Is(list) | Component::Where(list) => list
            .0
            .iter()
            .any(|selector| is_sensitive(&selector.components)),
        _ => false,
    })
}
