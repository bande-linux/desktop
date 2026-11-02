use std::sync::Arc;

use crate::collapse::{Collapsed, collapse};
use web_atoms::Atom;
use web_dom::{Document, NodeId, NodeKind};
use web_style::{
    ComputedStyle, ContentPart, Display, LineHeight, ListStyleType, Pseudo, ResetStyle, Styles,
    TableColumn, VerticalAlign,
};
use web_text::{
    FontKey, Fonts, LineAlign, Paragraph, RunStyle, ShapedParagraph, content_metrics, inline_extent,
};

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
    Grid(Vec<BlockBox>),
    Table(Box<Table>),
}

pub(crate) struct Table {
    pub(crate) captions: Vec<BlockBox>,
    pub(crate) rows: Vec<TableRow>,
    pub(crate) collapsed: Option<Box<Collapsed>>,
}

pub(crate) struct TableRow {
    pub(crate) node: Option<NodeId>,
    pub(crate) style: ComputedStyle,
    pub(crate) cells: Vec<TableCell>,
    pub(crate) group: Option<u32>,
}

#[derive(Default)]
pub(crate) struct Parts {
    pub(crate) groups: Vec<ComputedStyle>,
    pub(crate) columns: Vec<(ComputedStyle, Option<usize>)>,
    pub(crate) column_groups: Vec<ComputedStyle>,
}

pub(crate) struct TableCell {
    pub(crate) block: BlockBox,
    pub(crate) columns: u16,
    pub(crate) rows: u16,
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

#[derive(Default)]
pub struct BoxTree {
    pub(crate) root: Option<BlockBox>,
}

const MAX_DEPTH: u32 = 512;

struct Builder<'a> {
    document: &'a Document,
    styles: &'a Styles,
    fonts: &'a mut Fonts,
    depth: u32,
    counters: Vec<Counter>,
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
            counters: Vec::new(),
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

fn keep_collapsed(style: &mut ComputedStyle, old: &ResetStyle) {
    if style.reset.border != old.border
        || style.reset.border_color != old.border_color
        || style.reset.padding != old.padding
    {
        let reset = Arc::make_mut(&mut style.reset);
        reset.border = old.border;
        reset.border_color = old.border_color;
        reset.padding = old.padding;
    }
}

fn refresh(block: &mut BlockBox, styles: &Styles, parent: Option<&ComputedStyle>) {
    let old = block.style.reset.clone();
    match (block.node.and_then(|node| styles.get(node)), parent) {
        (Some(style), _) => block.style = style.clone(),
        (None, Some(parent)) => block.style = styles.anonymous(parent),
        (None, None) => {}
    }
    if let Contents::Table(table) = &block.contents
        && table.collapsed.is_some()
    {
        keep_collapsed(&mut block.style, &old);
    }
    let style = block.style.clone();
    match &mut block.contents {
        Contents::Blocks(children) => {
            for child in children {
                refresh(child, styles, Some(&style));
            }
        }
        Contents::Flex(children) | Contents::Grid(children) => {
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
        Contents::Table(table) => {
            for row in &mut table.rows {
                if let Some(row_style) = row.node.and_then(|node| styles.get(node)) {
                    row.style = row_style.clone();
                }
                for cell in &mut row.cells {
                    let old = cell.block.style.reset.clone();
                    refresh(&mut cell.block, styles, Some(&style));
                    if table.collapsed.is_some() {
                        keep_collapsed(&mut cell.block.style, &old);
                    }
                }
            }
            for caption in &mut table.captions {
                refresh(caption, styles, Some(&style));
            }
        }
    }
}

fn block_bytes(block: &BlockBox) -> usize {
    let marker = block.marker.as_deref().map_or(0, |marker| {
        size_of::<InlineContent>() + inline_bytes(marker)
    });
    marker
        + match &block.contents {
            Contents::Blocks(children) | Contents::Flex(children) | Contents::Grid(children) => {
                children.capacity() * size_of::<BlockBox>()
                    + children.iter().map(block_bytes).sum::<usize>()
            }
            Contents::Inline(inline) => size_of::<InlineContent>() + inline_bytes(inline),
            Contents::Replaced(_) => 0,
            Contents::Table(table) => {
                size_of::<Table>()
                    + table.rows.capacity() * size_of::<TableRow>()
                    + table
                        .rows
                        .iter()
                        .flat_map(|row| &row.cells)
                        .map(|cell| size_of::<TableCell>() + block_bytes(&cell.block))
                        .sum::<usize>()
                    + table
                        .captions
                        .iter()
                        .map(|caption| size_of::<BlockBox>() + block_bytes(caption))
                        .sum::<usize>()
            }
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

#[derive(Clone, Copy, Default)]
pub(crate) struct Shift {
    px: f32,
    align: LineAlign,
}

fn shifted(fonts: &mut Fonts, style: &ComputedStyle, tag: u32, shift: Shift) -> RunStyle {
    RunStyle {
        align: shift.align,
        ..run_style(fonts, style, tag, shift.px)
    }
}

fn resolve_shift(
    fonts: &mut Fonts,
    parent: &ComputedStyle,
    style: &ComputedStyle,
    outer: Shift,
    atomic: bool,
    group: usize,
) -> Shift {
    let vertical = style.reset.vertical_align;
    if outer.align.is_edge() {
        return Shift {
            px: outer.px + own_shift(style),
            ..outer
        };
    }
    let parent_metrics = |fonts: &mut Fonts| {
        let run = run_style(fonts, parent, 0, 0.0);
        content_metrics(fonts, run)
    };
    let own = |fonts: &mut Fonts| {
        let run = run_style(fonts, style, 0, 0.0);
        inline_extent(fonts, run)
    };
    match vertical {
        VerticalAlign::Top => Shift {
            px: 0.0,
            align: LineAlign::top(group),
        },
        VerticalAlign::Bottom => Shift {
            px: 0.0,
            align: LineAlign::bottom(group),
        },
        VerticalAlign::Middle | VerticalAlign::TextTop | VerticalAlign::TextBottom => {
            let (ascent, descent, x_height) = parent_metrics(fonts);
            let (align, target) = match vertical {
                VerticalAlign::Middle => (LineAlign::MIDDLE, outer.px + x_height / 2.0),
                VerticalAlign::TextTop => (LineAlign::TEXT_TOP, outer.px + ascent),
                _ => (LineAlign::TEXT_BOTTOM, outer.px - descent),
            };
            if atomic {
                return Shift { px: target, align };
            }
            let (above, below) = own(fonts);
            let px = match align {
                LineAlign::MIDDLE => target - (above - below) / 2.0,
                LineAlign::TEXT_TOP => target - above,
                _ => target + below,
            };
            Shift {
                px,
                align: LineAlign::BASELINE,
            }
        }
        _ => Shift {
            px: outer.px + own_shift(style),
            align: LineAlign::BASELINE,
        },
    }
}

pub(crate) fn run_style(
    fonts: &mut Fonts,
    style: &ComputedStyle,
    tag: u32,
    baseline_shift: f32,
) -> RunStyle {
    let inherited = &style.inherited;
    let names = fonts.family_names(&inherited.font_names);
    let size = inherited.font_size;
    RunStyle {
        font: FontKey {
            family: inherited.font_family,
            names,
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
        align: LineAlign::BASELINE,
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
        if matches!(style.display(), Display::Table | Display::InlineTable) {
            let mut style = style;
            let mut parts = Parts::default();
            let mut table = self.table(node, &style, &mut parts);
            if style.inherited.border_collapse {
                let outer = collapse(&mut table, &style, &parts);
                let reset = Arc::make_mut(&mut style.reset);
                reset.border = outer;
                reset.border_color = web_style::Sides::all(web_style::Color::TRANSPARENT);
                reset.padding = web_style::Sides::all(web_style::LengthPercentage::ZERO);
            }
            return BlockBox {
                node: Some(node),
                style,
                contents: Contents::Table(Box::new(table)),
                marker: None,
            };
        }
        let mut blocks = Vec::new();
        let mut pending = Pending::new(is_rtl(&style));
        pending
            .paragraph
            .set_nowrap(!style.inherited.white_space.wraps());
        let inside = style.inherited.list_inside;
        if inside
            && let Some(text) =
                ordinal.and_then(|ordinal| marker_text(style.inherited.list_style_type, ordinal))
        {
            let tag = pending.push(node, &style, ItemKind::Text);
            let mut run = run_style(self.fonts, &style, tag, 0.0);
            run.white_space = web_style::WhiteSpace::Pre;
            pending.paragraph.push_text(&format!("{text} "), run);
        }
        self.children(node, &mut blocks, &mut pending, Shift::default());
        let flex = matches!(style.display(), Display::Flex | Display::InlineFlex);
        let grid = matches!(style.display(), Display::Grid | Display::InlineGrid);
        let contents = if flex {
            self.flush(&mut blocks, pending, &style);
            Contents::Flex(blocks)
        } else if grid {
            self.flush(&mut blocks, pending, &style);
            Contents::Grid(blocks)
        } else if blocks.is_empty() {
            Contents::Inline(Box::new(self.finish(pending)))
        } else {
            self.flush(&mut blocks, pending, &style);
            Contents::Blocks(blocks)
        };
        let marker = ordinal
            .filter(|_| !inside)
            .and_then(|ordinal| self.marker(&style, ordinal));
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
        shift: Shift,
    ) {
        let scope = self.counters.len();
        let parent = self.styles.get(node).cloned();
        self.generated(node, Pseudo::Before, pending, shift);
        for child in self.document.children(node) {
            let Some(style) = self.styles.get(child) else {
                continue;
            };
            let ordinal = (self.document.kind(child) == Some(NodeKind::Element)
                && style.display() != Display::None)
                .then(|| self.apply_counters(child, style))
                .flatten();
            match self.document.kind(child) {
                Some(NodeKind::Text) => {
                    let text = self.document.text(child).unwrap_or_default();
                    let tag = pending.push(child, style, ItemKind::Text);
                    pending
                        .paragraph
                        .push_text(text, shifted(self.fonts, style, tag, shift));
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
                        let own = resolve_shift(
                            self.fonts,
                            parent.as_ref().unwrap_or(style),
                            style,
                            shift,
                            true,
                            child.index(),
                        );
                        pending
                            .paragraph
                            .push_atomic(index, shifted(self.fonts, style, tag, own));
                    }
                }
                Some(NodeKind::Element) => match style.display() {
                    Display::None => {}
                    Display::Inline | Display::Contents => {
                        let inner = resolve_shift(
                            self.fonts,
                            parent.as_ref().unwrap_or(style),
                            style,
                            shift,
                            false,
                            child.index(),
                        );
                        if self.document.html_element_name(child) == Some(Atom::BR) {
                            let tag = pending.push(child, style, ItemKind::Text);
                            pending
                                .paragraph
                                .push_break(shifted(self.fonts, style, tag, shift));
                        } else if style.display() == Display::Inline && decorated(style) {
                            let frame = InlineFrame::of(style);
                            let start = pending.push(child, style, ItemKind::Start);
                            pending.paragraph.push_spacer(
                                frame.start(),
                                shifted(self.fonts, style, start, inner),
                            );
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
                                pending.paragraph.push_spacer(
                                    frame.end(),
                                    shifted(self.fonts, style, end, inner),
                                );
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
                        let item_ordinal = ordinal.filter(|_| display == Display::ListItem);
                        self.depth += 1;
                        blocks.push(self.block(child, style.clone(), item_ordinal));
                        self.depth -= 1;
                    }
                },
                _ => {}
            }
        }
        self.generated(node, Pseudo::After, pending, shift);
        self.counters.truncate(scope);
    }

    fn counter(&self, name: &str) -> Option<usize> {
        self.counters
            .iter()
            .rposition(|counter| &*counter.name == name)
    }

    fn apply_counters(&mut self, element: NodeId, style: &ComputedStyle) -> Option<i32> {
        let changes = style.reset.counters.clone();
        let named = |list: Option<&[(Arc<str>, i32)]>, name: &str| {
            list.is_some_and(|list| list.iter().any(|(known, _)| &**known == name))
        };
        let html = self.document.html_element_name(element);
        if matches!(html, Some(Atom::OL | Atom::UL | Atom::MENU | Atom::DIR))
            && !named(
                changes.as_ref().map(|changes| &changes.reset[..]),
                LIST_ITEM,
            )
        {
            let reversed = html == Some(Atom::OL)
                && self.document.attribute(element, Atom::REVERSED).is_some();
            let start = self
                .document
                .attribute(element, Atom::START)
                .and_then(|start| start.trim().parse::<i32>().ok());
            let (value, step) = if reversed {
                let items = self
                    .document
                    .children(element)
                    .filter(|&child| {
                        self.styles
                            .get(child)
                            .is_some_and(|style| style.display() == Display::ListItem)
                    })
                    .count() as i32;
                (start.unwrap_or(items).saturating_add(1), -1)
            } else {
                (start.unwrap_or(1).saturating_sub(1), 1)
            };
            self.counters.push(Counter {
                name: Arc::from(LIST_ITEM),
                value,
                step,
            });
        }
        if let Some(changes) = &changes {
            for (name, value) in &changes.reset {
                self.counters.push(Counter {
                    name: name.clone(),
                    value: *value,
                    step: 1,
                });
            }
            for (name, value) in &changes.set {
                match self
                    .counter(name)
                    .and_then(|index| self.counters.get_mut(index))
                {
                    Some(counter) => counter.value = *value,
                    None => self.counters.push(Counter {
                        name: name.clone(),
                        value: *value,
                        step: 1,
                    }),
                }
            }
            for (name, value) in &changes.increment {
                match self
                    .counter(name)
                    .and_then(|index| self.counters.get_mut(index))
                {
                    Some(counter) => counter.value = counter.value.saturating_add(*value),
                    None => self.counters.push(Counter {
                        name: name.clone(),
                        value: *value,
                        step: 1,
                    }),
                }
            }
        }
        if style.display() != Display::ListItem {
            return None;
        }
        let explicit = named(
            changes.as_ref().map(|changes| &changes.increment[..]),
            LIST_ITEM,
        );
        let item_value = self
            .document
            .attribute(element, Atom::VALUE)
            .and_then(|value| value.trim().parse::<i32>().ok());
        let index = match self.counter(LIST_ITEM) {
            Some(index) => index,
            None => {
                self.counters.push(Counter {
                    name: Arc::from(LIST_ITEM),
                    value: 0,
                    step: 1,
                });
                self.counters.len() - 1
            }
        };
        let counter = self.counters.get_mut(index)?;
        match item_value {
            Some(value) if html == Some(Atom::LI) => counter.value = value,
            _ if !explicit => counter.value = counter.value.saturating_add(counter.step),
            _ => {}
        }
        Some(counter.value)
    }

    fn generated(&mut self, node: NodeId, pseudo: Pseudo, pending: &mut Pending, shift: Shift) {
        let Some(style) = self.styles.pseudo(node, pseudo).cloned() else {
            return;
        };
        let Some(parts) = style.reset.content.clone() else {
            return;
        };
        let mut text = String::new();
        for part in parts.iter() {
            match part {
                ContentPart::Text(part) => text.push_str(part),
                ContentPart::Counter(name, kind) => {
                    let value = self
                        .counter(name)
                        .and_then(|index| self.counters.get(index))
                        .map_or(0, |counter| counter.value);
                    text.push_str(&counter_text(value, *kind));
                }
                ContentPart::Counters(name, separator, kind) => {
                    let values: Vec<String> = self
                        .counters
                        .iter()
                        .filter(|counter| counter.name == *name)
                        .map(|counter| counter_text(counter.value, *kind))
                        .collect();
                    text.push_str(&if values.is_empty() {
                        counter_text(0, *kind)
                    } else {
                        values.join(separator)
                    });
                }
            }
        }
        let tag = pending.push(node, &style, ItemKind::Text);
        pending.paragraph.push_text(&text, {
            let own = match self.styles.get(node).cloned() {
                Some(parent) => {
                    resolve_shift(self.fonts, &parent, &style, shift, false, node.index())
                }
                None => shift,
            };
            shifted(self.fonts, &style, tag, own)
        });
    }

    fn table(&mut self, node: NodeId, style: &ComputedStyle, parts: &mut Parts) -> Table {
        let mut table = Table {
            captions: Vec::new(),
            rows: Vec::new(),
            collapsed: None,
        };
        let mut headers = Vec::new();
        let mut footers = Vec::new();
        self.depth += 1;
        for child in self.document.children(node).collect::<Vec<_>>() {
            let Some(child_style) = self.styles.get(child).cloned() else {
                continue;
            };
            if self.document.kind(child) != Some(NodeKind::Element) {
                continue;
            }
            match child_style.display() {
                Display::TableCaption => {
                    table.captions.push(self.block(child, child_style, None));
                }
                Display::TableHeaderGroup => {
                    self.row_group(child, child_style, &mut headers, parts)
                }
                Display::TableFooterGroup => {
                    self.row_group(child, child_style, &mut footers, parts)
                }
                Display::TableRowGroup => {
                    self.row_group(child, child_style, &mut table.rows, parts)
                }
                Display::TableRow => table.rows.push(self.row(child, child_style)),
                Display::None => self.column(child, child_style, parts),
                _ => {
                    let cell = self.cell(child, child_style);
                    match table.rows.last_mut().filter(|row| row.node.is_none()) {
                        Some(row) => row.cells.push(cell),
                        None => table.rows.push(TableRow {
                            node: None,
                            style: self.styles.anonymous(style),
                            cells: vec![cell],
                            group: None,
                        }),
                    }
                }
            }
        }
        self.depth -= 1;
        headers.append(&mut table.rows);
        headers.append(&mut footers);
        table.rows = headers;
        table
    }

    fn row_group(
        &mut self,
        group: NodeId,
        group_style: ComputedStyle,
        rows: &mut Vec<TableRow>,
        parts: &mut Parts,
    ) {
        let id = parts.groups.len() as u32;
        parts.groups.push(group_style);
        for child in self.document.children(group).collect::<Vec<_>>() {
            if self.document.kind(child) != Some(NodeKind::Element) {
                continue;
            }
            let Some(style) = self.styles.get(child).cloned() else {
                continue;
            };
            match style.display() {
                Display::TableRow => {
                    let mut row = self.row(child, style);
                    row.group = Some(id);
                    rows.push(row);
                }
                Display::None => {}
                _ => {
                    let cell = self.cell(child, style.clone());
                    rows.push(TableRow {
                        node: None,
                        style: self.styles.anonymous(&style),
                        cells: vec![cell],
                        group: Some(id),
                    });
                }
            }
        }
    }

    fn column(&self, node: NodeId, style: ComputedStyle, parts: &mut Parts) {
        let span = || {
            self.document
                .attribute(node, Atom::SPAN)
                .and_then(|span| span.trim().parse::<usize>().ok())
                .map_or(1, |span| span.clamp(1, 1000))
        };
        match style.reset.table_column {
            Some(TableColumn::Column) => {
                for _ in 0..span() {
                    parts.columns.push((style.clone(), None));
                }
            }
            Some(TableColumn::Group) => {
                let group = parts.column_groups.len();
                let start = parts.columns.len();
                for child in self.document.children(node) {
                    if let Some(child_style) = self.styles.get(child)
                        && self.document.kind(child) == Some(NodeKind::Element)
                        && child_style.reset.table_column == Some(TableColumn::Column)
                    {
                        let count = self
                            .document
                            .attribute(child, Atom::SPAN)
                            .and_then(|span| span.trim().parse::<usize>().ok())
                            .map_or(1, |span| span.clamp(1, 1000));
                        for _ in 0..count {
                            parts.columns.push((child_style.clone(), Some(group)));
                        }
                    }
                }
                if parts.columns.len() == start {
                    for _ in 0..span() {
                        parts
                            .columns
                            .push((self.styles.anonymous(&style), Some(group)));
                    }
                }
                parts.column_groups.push(style);
            }
            None => {}
        }
    }

    fn row(&mut self, row: NodeId, style: ComputedStyle) -> TableRow {
        let mut cells = Vec::new();
        for child in self.document.children(row).collect::<Vec<_>>() {
            if self.document.kind(child) != Some(NodeKind::Element) {
                continue;
            }
            let Some(child_style) = self.styles.get(child).cloned() else {
                continue;
            };
            if child_style.display() != Display::None {
                cells.push(self.cell(child, child_style));
            }
        }
        TableRow {
            node: Some(row),
            style,
            cells,
            group: None,
        }
    }

    fn cell(&mut self, cell: NodeId, style: ComputedStyle) -> TableCell {
        let span = |name: Atom, default: u32, max: u32| {
            self.document
                .attribute(cell, name)
                .and_then(|value| value.trim().parse::<u32>().ok())
                .map_or(default, |value| value.clamp(1, max)) as u16
        };
        let columns = span(Atom::COLSPAN, 1, 1000);
        let rows = span(Atom::ROWSPAN, 1, 65534);
        self.depth += 1;
        let block = self.block(cell, style, None);
        self.depth -= 1;
        TableCell {
            block,
            columns,
            rows,
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
        let text = marker_text(style.inherited.list_style_type, ordinal)?;
        let mut paragraph = Paragraph::new(is_rtl(style));
        let mut run = run_style(self.fonts, style, 0, 0.0);
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

const LIST_ITEM: &str = "list-item";

struct Counter {
    name: Arc<str>,
    value: i32,
    step: i32,
}

fn counter_text(value: i32, kind: ListStyleType) -> String {
    match kind {
        ListStyleType::None => String::new(),
        ListStyleType::Disc => "•".to_owned(),
        ListStyleType::Circle => "◦".to_owned(),
        ListStyleType::Square => "▪".to_owned(),
        ListStyleType::Decimal => value.to_string(),
        ListStyleType::LowerAlpha => alphabetic(value, b'a'),
        ListStyleType::UpperAlpha => alphabetic(value, b'A'),
        ListStyleType::LowerRoman => roman(value).to_ascii_lowercase(),
        ListStyleType::UpperRoman => roman(value),
    }
}

fn marker_text(kind: ListStyleType, ordinal: i32) -> Option<String> {
    match kind {
        ListStyleType::None => None,
        ListStyleType::Disc | ListStyleType::Circle | ListStyleType::Square => {
            Some(counter_text(ordinal, kind))
        }
        _ => Some(format!("{}.", counter_text(ordinal, kind))),
    }
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
