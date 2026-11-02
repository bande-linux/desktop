use std::sync::Arc;

use web_atoms::Atom;
use web_dom::{Document, NodeId, NodeKind};
use web_style::{ComputedStyle, Display, LineHeight, ListStyleType, Styles, VerticalAlign};
use web_text::{FontKey, Fonts, Paragraph, RunStyle, ShapedParagraph};

pub(crate) struct BlockBox {
    pub(crate) node: Option<NodeId>,
    pub(crate) style: ComputedStyle,
    pub(crate) contents: Contents,
    pub(crate) marker: Option<Box<InlineContent>>,
}

pub(crate) enum Contents {
    Blocks(Vec<BlockBox>),
    Inline(Box<InlineContent>),
    Replaced(Option<Arc<str>>),
    Flex(Vec<BlockBox>),
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ItemKind {
    Text,
    Start,
    End(u32),
    Atomic(u16),
}

pub(crate) struct InlineItem {
    pub(crate) node: NodeId,
    pub(crate) style: ComputedStyle,
    pub(crate) parent: Option<u32>,
    pub(crate) kind: ItemKind,
}

pub(crate) struct InlineFrame {
    pub(crate) margin: (f32, f32),
    pub(crate) border: (f32, f32),
    pub(crate) padding: (f32, f32),
}

impl InlineFrame {
    pub(crate) fn of(style: &ComputedStyle) -> Self {
        let reset = &style.reset;
        let px = |side: Option<web_style::LengthPercentage>| side.map_or(0.0, |side| side.px);
        let (margin, border, padding) = (
            (px(reset.margin.left), px(reset.margin.right)),
            (reset.border.left, reset.border.right),
            (reset.padding.left.px, reset.padding.right.px),
        );
        if style.is_rtl() {
            InlineFrame {
                margin: (margin.1, margin.0),
                border: (border.1, border.0),
                padding: (padding.1, padding.0),
            }
        } else {
            InlineFrame {
                margin,
                border,
                padding,
            }
        }
    }

    pub(crate) fn start(&self) -> f32 {
        self.margin.0 + self.border.0 + self.padding.0
    }

    pub(crate) fn end(&self) -> f32 {
        self.margin.1 + self.border.1 + self.padding.1
    }
}

fn decorated(style: &ComputedStyle) -> bool {
    let reset = &style.reset;
    let frame = InlineFrame::of(style);
    reset.background_color.a != 0
        || frame.start() != 0.0
        || frame.end() != 0.0
        || reset.border.top != 0.0
        || reset.border.bottom != 0.0
}

pub(crate) struct InlineContent {
    pub(crate) shaped: ShapedParagraph,
    pub(crate) items: Vec<InlineItem>,
    pub(crate) atomics: Vec<BlockBox>,
    pub(crate) out_of_flow: Vec<BlockBox>,
}

pub struct BoxTree {
    pub(crate) root: Option<BlockBox>,
}

const MAX_DEPTH: u32 = 512;

struct Builder<'a> {
    document: &'a Document,
    styles: &'a Styles,
    fonts: &'a mut Fonts,
    depth: u32,
}

struct Pending {
    paragraph: Paragraph,
    items: Vec<InlineItem>,
    open: Vec<u32>,
    atomics: Vec<BlockBox>,
    out_of_flow: Vec<BlockBox>,
}

impl Pending {
    fn new(rtl: bool) -> Self {
        Self {
            paragraph: Paragraph::new(rtl),
            items: Vec::new(),
            open: Vec::new(),
            atomics: Vec::new(),
            out_of_flow: Vec::new(),
        }
    }

    fn push(&mut self, node: NodeId, style: &ComputedStyle, kind: ItemKind) -> u32 {
        let tag = self.items.len() as u32;
        self.items.push(InlineItem {
            node,
            style: style.clone(),
            parent: self.open.last().copied(),
            kind,
        });
        tag
    }
}

impl BoxTree {
    pub fn build(document: &Document, styles: &Styles, fonts: &mut Fonts) -> Self {
        let mut builder = Builder {
            document,
            styles,
            fonts,
            depth: 0,
        };
        let root = document
            .children(NodeId::DOCUMENT)
            .find(|&child| document.kind(child) == Some(NodeKind::Element))
            .and_then(|root| {
                let style = styles.get(root)?.clone();
                Some(builder.block(root, style, None))
            });
        BoxTree { root }
    }

    pub fn heap_bytes(&self) -> usize {
        self.root.as_ref().map_or(0, block_bytes)
    }

    pub fn refresh_styles(&mut self, styles: &Styles) {
        if let Some(root) = &mut self.root {
            refresh(root, styles, None);
        }
    }
}

fn refresh(block: &mut BlockBox, styles: &Styles, parent: Option<&ComputedStyle>) {
    match (block.node.and_then(|node| styles.get(node)), parent) {
        (Some(style), _) => block.style = style.clone(),
        (None, Some(parent)) => block.style = styles.anonymous(parent),
        (None, None) => {}
    }
    let style = block.style.clone();
    match &mut block.contents {
        Contents::Blocks(children) => {
            for child in children {
                refresh(child, styles, Some(&style));
            }
        }
        Contents::Flex(children) => {
            for child in children {
                refresh(child, styles, Some(&style));
            }
        }
        Contents::Inline(inline) => {
            for item in &mut inline.items {
                if let Some(style) = styles.get(item.node) {
                    item.style = style.clone();
                }
            }
            for child in inline.atomics.iter_mut().chain(&mut inline.out_of_flow) {
                refresh(child, styles, Some(&style));
            }
        }
        Contents::Replaced(_) => {}
    }
}

fn block_bytes(block: &BlockBox) -> usize {
    let marker = block.marker.as_deref().map_or(0, |marker| {
        size_of::<InlineContent>() + inline_bytes(marker)
    });
    marker
        + match &block.contents {
            Contents::Blocks(children) | Contents::Flex(children) => {
                children.capacity() * size_of::<BlockBox>()
                    + children.iter().map(block_bytes).sum::<usize>()
            }
            Contents::Inline(inline) => size_of::<InlineContent>() + inline_bytes(inline),
            Contents::Replaced(_) => 0,
        }
}

fn inline_bytes(inline: &InlineContent) -> usize {
    inline.shaped.heap_bytes()
        + inline.items.capacity() * size_of::<InlineItem>()
        + inline
            .atomics
            .iter()
            .chain(&inline.out_of_flow)
            .map(|block| size_of::<BlockBox>() + block_bytes(block))
            .sum::<usize>()
}

pub(crate) fn is_rtl(style: &ComputedStyle) -> bool {
    style.inherited.direction == web_style::Direction::Rtl
}

fn own_shift(style: &ComputedStyle) -> f32 {
    let size = style.inherited.font_size;
    match style.reset.vertical_align {
        VerticalAlign::Sub => -0.25 * size,
        VerticalAlign::Super => 0.5 * size,
        VerticalAlign::Length(length) => length.resolve(style.inherited.font_size * 1.2),
        _ => 0.0,
    }
}

pub(crate) fn run_style(style: &ComputedStyle, tag: u32, baseline_shift: f32) -> RunStyle {
    let inherited = &style.inherited;
    let size = inherited.font_size;
    RunStyle {
        font: FontKey {
            family: inherited.font_family,
            weight: inherited.font_weight,
            italic: inherited.italic,
        },
        size,
        line_height: match inherited.line_height {
            LineHeight::Normal => None,
            LineHeight::Number(factor) => Some(factor * size),
            LineHeight::Px(px) => Some(px),
        },
        baseline_shift,
        white_space: inherited.white_space,
        tag,
    }
}

impl Builder<'_> {
    fn block(&mut self, node: NodeId, style: ComputedStyle, ordinal: Option<i32>) -> BlockBox {
        if self.document.html_element_name(node) == Some(Atom::IMG) {
            let source = self.document.attribute(node, Atom::SRC).map(Arc::from);
            return BlockBox {
                node: Some(node),
                style,
                contents: Contents::Replaced(source),
                marker: None,
            };
        }
        let mut blocks = Vec::new();
        let mut pending = Pending::new(is_rtl(&style));
        self.children(node, &mut blocks, &mut pending, 0.0);
        let flex = matches!(style.display(), Display::Flex | Display::InlineFlex);
        let contents = if flex {
            self.flush(&mut blocks, pending, &style);
            Contents::Flex(blocks)
        } else if blocks.is_empty() {
            Contents::Inline(Box::new(self.finish(pending)))
        } else {
            self.flush(&mut blocks, pending, &style);
            Contents::Blocks(blocks)
        };
        let marker = ordinal.and_then(|ordinal| self.marker(&style, ordinal));
        BlockBox {
            node: Some(node),
            style,
            contents,
            marker,
        }
    }

    fn children(
        &mut self,
        node: NodeId,
        blocks: &mut Vec<BlockBox>,
        pending: &mut Pending,
        shift: f32,
    ) {
        let mut ordinal = list_start(self.document, node);
        let reversed = self.document.attribute(node, Atom::REVERSED).is_some();
        for child in self.document.children(node) {
            let Some(style) = self.styles.get(child) else {
                continue;
            };
            match self.document.kind(child) {
                Some(NodeKind::Text) => {
                    let text = self.document.text(child).unwrap_or_default();
                    let tag = pending.push(child, style, ItemKind::Text);
                    pending
                        .paragraph
                        .push_text(text, run_style(style, tag, shift));
                }
                Some(NodeKind::Element) if self.depth >= MAX_DEPTH => {}
                Some(NodeKind::Element) if style.display() == Display::None => {}
                Some(NodeKind::Element)
                    if style.is_out_of_flow() || style.reset.float != web_style::Float::None =>
                {
                    self.depth += 1;
                    let block = self.block(child, style.clone(), None);
                    self.depth -= 1;
                    pending.out_of_flow.push(block);
                }
                Some(NodeKind::Element)
                    if style.display().is_inline_level() && style.display() != Display::Inline
                        || (style.display() == Display::Inline
                            && self.document.html_element_name(child) == Some(Atom::IMG)) =>
                {
                    self.depth += 1;
                    let block = self.block(child, style.clone(), None);
                    self.depth -= 1;
                    let index = u16::try_from(pending.atomics.len()).unwrap_or(u16::MAX);
                    if index < u16::MAX {
                        pending.atomics.push(block);
                        let tag = pending.push(child, style, ItemKind::Atomic(index));
                        pending
                            .paragraph
                            .push_atomic(index, run_style(style, tag, shift + own_shift(style)));
                    }
                }
                Some(NodeKind::Element) => match style.display() {
                    Display::None => {}
                    Display::Inline | Display::Contents => {
                        let inner = shift + own_shift(style);
                        if self.document.html_element_name(child) == Some(Atom::BR) {
                            let tag = pending.push(child, style, ItemKind::Text);
                            pending.paragraph.push_break(run_style(style, tag, shift));
                        } else if style.display() == Display::Inline && decorated(style) {
                            let frame = InlineFrame::of(style);
                            let start = pending.push(child, style, ItemKind::Start);
                            pending
                                .paragraph
                                .push_spacer(frame.start(), run_style(style, start, inner));
                            pending.open.push(start);
                            self.depth += 1;
                            self.children(child, blocks, pending, inner);
                            self.depth -= 1;
                            let same = pending.items.get(start as usize).is_some_and(|item| {
                                item.node == child && item.kind == ItemKind::Start
                            });
                            if same {
                                pending.open.pop();
                                let end = pending.push(child, style, ItemKind::End(start));
                                pending
                                    .paragraph
                                    .push_spacer(frame.end(), run_style(style, end, inner));
                            }
                        } else {
                            self.depth += 1;
                            self.children(child, blocks, pending, inner);
                            self.depth -= 1;
                        }
                    }
                    display => {
                        let parent_style = self.styles.get(node).cloned();
                        if let Some(parent_style) = parent_style {
                            let taken = std::mem::replace(
                                pending,
                                Pending::new(pending.paragraph.is_rtl()),
                            );
                            self.flush(blocks, taken, &parent_style);
                        }
                        let item_ordinal = (display == Display::ListItem).then_some(ordinal);
                        if display == Display::ListItem {
                            ordinal += if reversed { -1 } else { 1 };
                        }
                        self.depth += 1;
                        blocks.push(self.block(child, style.clone(), item_ordinal));
                        self.depth -= 1;
                    }
                },
                _ => {}
            }
        }
    }

    fn flush(&mut self, blocks: &mut Vec<BlockBox>, mut pending: Pending, style: &ComputedStyle) {
        if pending.paragraph.is_blank() && !pending.paragraph.has_atomics() {
            blocks.append(&mut pending.out_of_flow);
            return;
        }
        let anonymous = self.styles.anonymous(style);
        blocks.push(BlockBox {
            node: None,
            style: anonymous,
            contents: Contents::Inline(Box::new(self.finish(pending))),
            marker: None,
        });
    }

    fn finish(&mut self, pending: Pending) -> InlineContent {
        InlineContent {
            shaped: pending.paragraph.shape(self.fonts),
            items: pending.items,
            atomics: pending.atomics,
            out_of_flow: pending.out_of_flow,
        }
    }

    fn marker(&mut self, style: &ComputedStyle, ordinal: i32) -> Option<Box<InlineContent>> {
        let text = match style.inherited.list_style_type {
            ListStyleType::None => return None,
            ListStyleType::Disc => "•".to_owned(),
            ListStyleType::Circle => "◦".to_owned(),
            ListStyleType::Square => "▪".to_owned(),
            ListStyleType::Decimal => format!("{ordinal}."),
            ListStyleType::LowerAlpha => format!("{}.", alphabetic(ordinal, b'a')),
            ListStyleType::UpperAlpha => format!("{}.", alphabetic(ordinal, b'A')),
            ListStyleType::LowerRoman => format!("{}.", roman(ordinal).to_ascii_lowercase()),
            ListStyleType::UpperRoman => format!("{}.", roman(ordinal)),
        };
        let mut paragraph = Paragraph::new(is_rtl(style));
        let mut run = run_style(style, 0, 0.0);
        run.white_space = web_style::WhiteSpace::Pre;
        paragraph.push_text(&text, run);
        Some(Box::new(InlineContent {
            shaped: paragraph.shape(self.fonts),
            items: Vec::new(),
            atomics: Vec::new(),
            out_of_flow: Vec::new(),
        }))
    }
}

fn list_start(document: &Document, node: NodeId) -> i32 {
    document
        .attribute(node, Atom::START)
        .and_then(|start| start.trim().parse().ok())
        .unwrap_or(1)
}

fn alphabetic(ordinal: i32, first: u8) -> String {
    if ordinal < 1 {
        return ordinal.to_string();
    }
    let mut value = ordinal as u32;
    let mut letters = Vec::new();
    while value > 0 {
        value -= 1;
        letters.push(char::from(first + (value % 26) as u8));
        value /= 26;
    }
    letters.iter().rev().collect()
}

fn roman(ordinal: i32) -> String {
    if !(1..4000).contains(&ordinal) {
        return ordinal.to_string();
    }
    const NUMERALS: [(i32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut value = ordinal;
    let mut text = String::new();
    for (amount, numeral) in NUMERALS {
        while value >= amount {
            text.push_str(numeral);
            value -= amount;
        }
    }
    text
}
