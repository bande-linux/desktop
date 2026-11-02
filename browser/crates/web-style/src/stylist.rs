use std::collections::HashMap;

use web_atoms::{Atom, AtomTable, Namespace};
use web_css::media::{MediaEnvironment, MediaList};
use web_css::properties::DeclarationBlock;
use web_css::selectors::{Combinator, Component, PseudoClass, Selector, SelectorList};
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
    buckets: HashMap<Key, Vec<u32>>,
    pub(crate) namespaces: Vec<(Atom, Namespace)>,
    pub(crate) hover_subjects: Vec<Key>,
    pub(crate) hover_ancestors: Vec<Key>,
    pub(crate) viewport: Viewport,
    layers: Vec<Box<[Atom]>>,
}

struct Builder<'a> {
    stylist: &'a mut Stylist,
    environment: MediaEnvironment,
}

impl Stylist {
    pub fn new(atoms: &mut AtomTable, sheets: Vec<Stylesheet>, viewport: Viewport) -> Self {
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
            namespaces,
            hover_subjects: Vec::new(),
            hover_ancestors: Vec::new(),
            viewport,
            layers: Vec::new(),
        };
        let mut builder = Builder {
            stylist: &mut stylist,
            environment: environment(viewport),
        };
        let user_agent = parse_stylesheet(UA_SHEET, atoms);
        builder.add_rules(user_agent.rules, Origin::UserAgent, UNLAYERED, None);
        for sheet in sheets {
            builder.add_rules(sheet.rules, Origin::Author, UNLAYERED, None);
        }
        stylist
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
            .flatten()
            .copied()
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

    fn add_rules(
        &mut self,
        rules: Vec<CssRule>,
        origin: Origin,
        layer: u16,
        media: Option<&MediaList>,
    ) {
        if media.is_some_and(|media| !media.matches(&self.environment)) {
            return;
        }
        for rule in rules {
            match rule {
                CssRule::Style(style) => {
                    let style = *style;
                    let index = self.stylist.rules.len() as u32;
                    self.stylist.rules.push(RuleData {
                        block: style.declarations,
                        origin,
                        layer,
                    });
                    for selector in 0..style.selectors.0.len() {
                        self.add_selector(&style.selectors, selector as u32, index);
                    }
                    self.add_rules(style.rules, origin, layer, None);
                }
                CssRule::Media(list, rules) => {
                    if list.matches(&self.environment) {
                        self.add_rules(rules, origin, layer, None);
                    }
                }
                CssRule::Supports(true, rules) | CssRule::Container(_, _, rules) => {
                    self.add_rules(rules, origin, layer, None)
                }
                CssRule::Layer(name, rules) => {
                    let parent = self.layer_path(layer);
                    let layer = self.layer(parent.as_deref(), name.as_ref());
                    self.add_rules(rules, origin, layer, None);
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
        let key = subject_key(&selector.components);
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
        let entry = self.stylist.entries.len() as u32;
        self.stylist.entries.push(Entry {
            sensitive: is_sensitive(&selector.components),
            list: list.clone(),
            index,
            rule,
        });
        self.stylist.buckets.entry(key).or_default().push(entry);
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
