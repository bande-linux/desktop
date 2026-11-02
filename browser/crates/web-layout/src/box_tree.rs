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
}

pub(crate) struct InlineItem {
    pub(crate) node: NodeId,
    pub(crate) style: ComputedStyle,
}

pub(crate) struct InlineContent {
    pub(crate) paragraph: Paragraph,
    pub(crate) shaped: ShapedParagraph,
    pub(crate) items: Vec<InlineItem>,
}

pub struct BoxTree {
    pub(crate) root: Option<BlockBox>,
}

struct Builder<'a> {
    document: &'a Document,
    styles: &'a Styles,
    fonts: &'a mut Fonts,
}

struct Pending {
    paragraph: Paragraph,
    items: Vec<InlineItem>,
}

impl Pending {
    fn new() -> Self {
        Self {
            paragraph: Paragraph::new(),
            items: Vec::new(),
        }
    }
}

impl BoxTree {
    pub fn build(document: &Document, styles: &Styles, fonts: &mut Fonts) -> Self {
        let mut builder = Builder {
            document,
            styles,
            fonts,
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
}

fn block_bytes(block: &BlockBox) -> usize {
    let marker = block.marker.as_deref().map_or(0, |marker| {
        size_of::<InlineContent>() + inline_bytes(marker)
    });
    marker
        + match &block.contents {
            Contents::Blocks(children) => {
                children.capacity() * size_of::<BlockBox>()
                    + children.iter().map(block_bytes).sum::<usize>()
            }
            Contents::Inline(inline) => size_of::<InlineContent>() + inline_bytes(inline),
        }
}

fn inline_bytes(inline: &InlineContent) -> usize {
    inline.paragraph.text().len()
        + inline.shaped.heap_bytes()
        + inline.items.capacity() * size_of::<InlineItem>()
}

pub(crate) fn run_style(style: &ComputedStyle, tag: u32) -> RunStyle {
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
        baseline_shift: match style.reset.vertical_align {
            VerticalAlign::Baseline => 0.0,
            VerticalAlign::Sub => -0.25 * size,
            VerticalAlign::Super => 0.5 * size,
        },
        white_space: inherited.white_space,
        tag,
    }
}

impl Builder<'_> {
    fn block(&mut self, node: NodeId, style: ComputedStyle, ordinal: Option<i32>) -> BlockBox {
        let mut blocks = Vec::new();
        let mut pending = Pending::new();
        self.children(node, &mut blocks, &mut pending);
        let contents = if blocks.is_empty() {
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

    fn children(&mut self, node: NodeId, blocks: &mut Vec<BlockBox>, pending: &mut Pending) {
        let mut ordinal = list_start(self.document, node);
        let reversed = self.document.attribute(node, Atom::REVERSED).is_some();
        for child in self.document.children(node) {
            let Some(style) = self.styles.get(child) else {
                continue;
            };
            match self.document.kind(child) {
                Some(NodeKind::Text) => {
                    let text = self.document.text(child).unwrap_or_default();
                    let tag = pending.items.len() as u32;
                    pending.paragraph.push_text(text, run_style(style, tag));
                    pending.items.push(InlineItem {
                        node: child,
                        style: style.clone(),
                    });
                }
                Some(NodeKind::Element) => match style.display() {
                    Display::None => {}
                    Display::Inline => {
                        if self.document.html_element_name(child) == Some(Atom::BR) {
                            let tag = pending.items.len() as u32;
                            pending.paragraph.push_break(run_style(style, tag));
                            pending.items.push(InlineItem {
                                node: child,
                                style: style.clone(),
                            });
                        } else {
                            self.children(child, blocks, pending);
                        }
                    }
                    display => {
                        let parent_style = self.styles.get(node).cloned();
                        if let Some(parent_style) = parent_style {
                            let taken = std::mem::replace(pending, Pending::new());
                            self.flush(blocks, taken, &parent_style);
                        }
                        let item_ordinal = (display == Display::ListItem).then_some(ordinal);
                        if display == Display::ListItem {
                            ordinal += if reversed { -1 } else { 1 };
                        }
                        blocks.push(self.block(child, style.clone(), item_ordinal));
                    }
                },
                _ => {}
            }
        }
    }

    fn flush(&mut self, blocks: &mut Vec<BlockBox>, pending: Pending, style: &ComputedStyle) {
        if pending.paragraph.is_blank() {
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
        let shaped = pending.paragraph.shape(self.fonts);
        InlineContent {
            paragraph: pending.paragraph,
            shaped,
            items: pending.items,
        }
    }

    fn marker(&mut self, style: &ComputedStyle, ordinal: i32) -> Option<Box<InlineContent>> {
        let text = match style.inherited.list_style_type {
            ListStyleType::None => return None,
            ListStyleType::Disc => "•".to_owned(),
            ListStyleType::Circle => "◦".to_owned(),
            ListStyleType::Square => "▪".to_owned(),
            ListStyleType::Decimal => format!("{ordinal}."),
        };
        let mut paragraph = Paragraph::new();
        let mut run = run_style(style, 0);
        run.white_space = web_style::WhiteSpace::Pre;
        paragraph.push_text(&text, run);
        let shaped = paragraph.shape(self.fonts);
        Some(Box::new(InlineContent {
            paragraph,
            shaped,
            items: Vec::new(),
        }))
    }
}

fn list_start(document: &Document, node: NodeId) -> i32 {
    document
        .attribute(node, Atom::START)
        .and_then(|start| start.trim().parse().ok())
        .unwrap_or(1)
}
