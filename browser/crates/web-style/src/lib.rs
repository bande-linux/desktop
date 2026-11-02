mod defaults;

use std::collections::HashMap;
use std::sync::Arc;

use web_atoms::Atom;
use web_dom::{Document, Namespace, NodeId, NodeKind};

pub use defaults::Declaration;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const TRANSPARENT: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Display {
    None,
    Block,
    Inline,
    ListItem,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FontFamily {
    Serif,
    SansSerif,
    Monospace,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WhiteSpace {
    Normal,
    Pre,
    Nowrap,
    PreWrap,
    PreLine,
}

impl WhiteSpace {
    pub fn collapses_spaces(self) -> bool {
        matches!(self, Self::Normal | Self::Nowrap | Self::PreLine)
    }

    pub fn preserves_newlines(self) -> bool {
        !matches!(self, Self::Normal | Self::Nowrap)
    }

    pub fn wraps(self) -> bool {
        !matches!(self, Self::Pre | Self::Nowrap)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TextAlign {
    Start,
    Left,
    Right,
    Center,
    Justify,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ListStyleType {
    None,
    Disc,
    Circle,
    Square,
    Decimal,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LineHeight {
    Normal,
    Number(f32),
    Px(f32),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum VerticalAlign {
    Baseline,
    Sub,
    Super,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct TextDecorations {
    pub underline: Option<Color>,
    pub line_through: Option<Color>,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Sides {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

#[derive(Clone, PartialEq, Debug)]
pub struct InheritedStyle {
    pub color: Color,
    pub font_family: FontFamily,
    pub font_size: f32,
    pub font_weight: u16,
    pub italic: bool,
    pub line_height: LineHeight,
    pub white_space: WhiteSpace,
    pub text_align: TextAlign,
    pub list_style_type: ListStyleType,
    pub decorations: TextDecorations,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ResetStyle {
    pub display: Display,
    pub margin: Sides,
    pub padding: Sides,
    pub border: Sides,
    pub border_color: Color,
    pub background_color: Color,
    pub vertical_align: VerticalAlign,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ComputedStyle {
    pub inherited: Arc<InheritedStyle>,
    pub reset: Arc<ResetStyle>,
}

impl ComputedStyle {
    pub fn initial() -> Self {
        Self {
            inherited: Arc::new(InheritedStyle {
                color: Color::BLACK,
                font_family: FontFamily::Serif,
                font_size: MEDIUM,
                font_weight: 400,
                italic: false,
                line_height: LineHeight::Normal,
                white_space: WhiteSpace::Normal,
                text_align: TextAlign::Start,
                list_style_type: ListStyleType::Disc,
                decorations: TextDecorations::default(),
            }),
            reset: Arc::new(INITIAL_RESET),
        }
    }

    pub fn display(&self) -> Display {
        self.reset.display
    }

    pub fn line_height_px(&self, normal: f32) -> f32 {
        match self.inherited.line_height {
            LineHeight::Normal => normal,
            LineHeight::Number(factor) => factor * self.inherited.font_size,
            LineHeight::Px(px) => px,
        }
    }

    fn derived(&self, declarations: &[Declaration], initial_reset: &Arc<ResetStyle>) -> Self {
        let mut inherited = (*self.inherited).clone();
        let mut reset = INITIAL_RESET;
        for declaration in declarations {
            declaration.apply(&mut inherited, &mut reset, &self.inherited);
        }
        Self {
            inherited: if inherited == *self.inherited {
                self.inherited.clone()
            } else {
                Arc::new(inherited)
            },
            reset: if reset == **initial_reset {
                initial_reset.clone()
            } else {
                Arc::new(reset)
            },
        }
    }
}

const MEDIUM: f32 = 16.0;

const INITIAL_RESET: ResetStyle = ResetStyle {
    display: Display::Inline,
    margin: Sides {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    },
    padding: Sides {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    },
    border: Sides {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    },
    border_color: Color::BLACK,
    background_color: Color::TRANSPARENT,
    vertical_align: VerticalAlign::Baseline,
};

const UNSTYLED: u32 = u32::MAX;

pub struct Styles {
    per_node: Vec<u32>,
    pool: Vec<ComputedStyle>,
}

impl Styles {
    pub fn compute(document: &Document) -> Self {
        let root = ComputedStyle::initial();
        let initial_reset = root.reset.clone();
        let mut styles = Styles {
            per_node: vec![UNSTYLED; document.len()],
            pool: vec![root],
        };
        let mut sharing: HashMap<(u32, defaults::RuleKey), u32> = HashMap::new();
        let mut stack: Vec<(NodeId, u32)> = document
            .children(NodeId::DOCUMENT)
            .map(|child| (child, 0))
            .collect();
        stack.reverse();
        while let Some((node, parent)) = stack.pop() {
            let style = match document.kind(node) {
                Some(NodeKind::Element) => {
                    let key = defaults::rule_key(document, node);
                    let pool = &mut styles.pool;
                    match sharing.get(&(parent, key)) {
                        Some(&shared) => shared,
                        None => {
                            let Some(parent_style) = pool.get(parent as usize) else {
                                continue;
                            };
                            let style = parent_style
                                .derived(defaults::declarations(key), &initial_reset);
                            pool.push(style);
                            let index = (pool.len() - 1) as u32;
                            sharing.insert((parent, key), index);
                            index
                        }
                    }
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
            let start = stack.len();
            stack.extend(document.children(node).map(|child| (child, style)));
            stack[start..].reverse();
        }
        styles
    }

    pub fn get(&self, node: NodeId) -> Option<&ComputedStyle> {
        let index = *self.per_node.get(node.index())?;
        self.pool.get(index as usize)
    }

    pub fn distinct(&self) -> usize {
        self.pool.len()
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
        self.per_node.capacity() * size_of::<u32>()
            + self.pool.capacity() * size_of::<ComputedStyle>()
            + groups
    }
}

pub(crate) fn is_html(document: &Document, node: NodeId, name: Atom) -> bool {
    document.element_name(node) == Some((Namespace::Html, name))
}
