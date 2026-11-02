use std::ops::Range;

use web_dom::NodeId;
use web_style::{Color, ComputedStyle, Sides, TextAlign, TextDecorations};
use web_text::{FontId, Fonts, Glyph, Lines};

use crate::box_tree::{BlockBox, BoxTree, Contents, InlineContent, is_rtl, run_style};

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, PartialEq, Debug)]
pub struct BoxFragment {
    pub node: Option<NodeId>,
    pub rect: Rect,
    pub background: Color,
    pub border: Sides,
    pub border_color: Sides<Color>,
    pub children: Vec<Fragment>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct TextFragment {
    pub node: Option<NodeId>,
    pub rect: Rect,
    pub baseline: f32,
    pub font: FontId,
    pub size: f32,
    pub color: Color,
    pub background: Color,
    pub decorations: TextDecorations,
    pub glyphs: Range<u32>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Fragment {
    Box(BoxFragment),
    Text(TextFragment),
}

#[derive(Clone, PartialEq, Debug)]
pub struct FragmentTree {
    pub root: BoxFragment,
    pub glyphs: Vec<Glyph>,
}

impl FragmentTree {
    pub fn height(&self) -> f32 {
        self.root.rect.y + self.root.rect.height
    }

    pub fn heap_bytes(&self) -> usize {
        self.glyphs.capacity() * size_of::<Glyph>() + box_bytes(&self.root)
    }
}

fn box_bytes(fragment: &BoxFragment) -> usize {
    fragment.children.capacity() * size_of::<Fragment>()
        + fragment
            .children
            .iter()
            .map(|child| match child {
                Fragment::Box(child) => box_bytes(child),
                Fragment::Text(_) => 0,
            })
            .sum::<usize>()
}

#[derive(Clone, Copy, Default)]
struct Margins {
    positive: f32,
    negative: f32,
}

impl Margins {
    fn of(value: f32) -> Self {
        let mut margins = Self::default();
        margins.adjoin(value);
        margins
    }

    fn adjoin(&mut self, value: f32) {
        self.positive = self.positive.max(value);
        self.negative = self.negative.min(value);
    }

    fn adjoin_all(&mut self, other: Margins) {
        self.adjoin(other.positive);
        self.adjoin(other.negative);
    }

    fn resolve(self) -> f32 {
        self.positive + self.negative
    }
}

struct Laid {
    fragment: BoxFragment,
    top: Margins,
    bottom: Margins,
    through: bool,
}

struct Flow<'a> {
    fonts: &'a mut Fonts,
    glyphs: Vec<Glyph>,
}

impl BoxTree {
    pub fn layout(&self, width: f32, fonts: &mut Fonts) -> FragmentTree {
        let mut flow = Flow {
            fonts,
            glyphs: Vec::new(),
        };
        let root = match &self.root {
            Some(root) => {
                let laid = flow.block(root, width.max(0.0), true);
                let mut fragment = laid.fragment;
                fragment.rect.y = laid.top.resolve();
                fragment
            }
            None => BoxFragment {
                node: None,
                rect: Rect::default(),
                background: Color::TRANSPARENT,
                border: Sides::default(),
                border_color: Sides::all(Color::TRANSPARENT),
                children: Vec::new(),
            },
        };
        FragmentTree {
            root,
            glyphs: flow.glyphs,
        }
    }
}

impl Flow<'_> {
    fn block(&mut self, block: &BlockBox, containing_width: f32, root: bool) -> Laid {
        let reset = &block.style.reset;
        let border = reset.border;
        let padding = reset
            .padding
            .map(|side| side.resolve(containing_width).max(0.0));
        let resolve = |side: Option<web_style::LengthPercentage>| {
            side.map(|side| side.resolve(containing_width))
        };
        let (mut margin_left, mut margin_right) =
            (resolve(reset.margin.left), resolve(reset.margin.right));
        let frame = border.left + border.right + padding.left + padding.right;
        let specified = reset.width.map(|width| width.resolve(containing_width));
        let maximum = reset.max_width.map(|width| width.resolve(containing_width));
        let minimum = reset.min_width.resolve(containing_width);
        let auto_width =
            (containing_width - margin_left.unwrap_or(0.0) - margin_right.unwrap_or(0.0) - frame)
                .max(0.0);
        let mut content_width = specified.unwrap_or(auto_width);
        if let Some(maximum) = maximum {
            content_width = content_width.min(maximum);
        }
        content_width = content_width.max(minimum).max(0.0);
        if specified.is_some() || maximum.is_some_and(|maximum| maximum < auto_width) {
            let rest = containing_width - content_width - frame;
            match (margin_left, margin_right) {
                (None, None) => {
                    margin_left = Some((rest / 2.0).max(0.0));
                    margin_right = Some(rest - (rest / 2.0).max(0.0));
                }
                (None, Some(right)) => margin_left = Some(rest - right),
                (Some(left), None) => margin_right = Some(rest - left),
                (Some(_), Some(_)) => {}
            }
        }
        let margin = web_style::Sides {
            top: resolve(reset.margin.top).unwrap_or(0.0),
            right: margin_right.unwrap_or(0.0),
            bottom: resolve(reset.margin.bottom).unwrap_or(0.0),
            left: margin_left.unwrap_or(0.0),
        };
        let width = content_width + frame;
        let content_x = border.left + padding.left;
        let content_y = border.top + padding.top;
        let top_adjoins = !root && border.top == 0.0 && padding.top == 0.0;
        let bottom_adjoins = !root && border.bottom == 0.0 && padding.bottom == 0.0;
        let mut top = Margins::of(margin.top);
        let mut children = Vec::new();
        let mut y = 0.0;
        let mut through = top_adjoins && bottom_adjoins;
        let mut pending = Margins::default();
        match &block.contents {
            Contents::Blocks(blocks) => {
                let mut at_top = top_adjoins;
                for child in blocks {
                    let laid = self.block(child, content_width, false);
                    if at_top {
                        top.adjoin_all(laid.top);
                        if laid.through {
                            top.adjoin_all(laid.bottom);
                            continue;
                        }
                    } else {
                        pending.adjoin_all(laid.top);
                        if laid.through {
                            pending.adjoin_all(laid.bottom);
                            continue;
                        }
                        y += pending.resolve();
                    }
                    at_top = false;
                    through = false;
                    let mut fragment = laid.fragment;
                    fragment.rect.x += content_x;
                    fragment.rect.y = content_y + y;
                    y += fragment.rect.height;
                    pending = laid.bottom;
                    children.push(Fragment::Box(fragment));
                }
            }
            Contents::Inline(inline) => {
                let lines = inline.shaped.lines(
                    self.fonts,
                    content_width,
                    run_style(&block.style, u32::MAX, 0.0),
                    block.style.inherited.text_align == TextAlign::Justify,
                );
                if !lines.lines.is_empty() {
                    through = false;
                    let rtl = is_rtl(&block.style);
                    let align = match block.style.inherited.text_align {
                        TextAlign::Start | TextAlign::Justify if rtl => TextAlign::Right,
                        TextAlign::End if !rtl => TextAlign::Right,
                        TextAlign::End => TextAlign::Left,
                        align => align,
                    };
                    self.inline(
                        inline,
                        &lines,
                        (content_x, content_y),
                        content_width,
                        align,
                        &mut children,
                    );
                    y = lines.height;
                }
            }
        }
        if let Some(height) = reset.height.filter(|height| height.percent == 0.0) {
            y = height.px.max(0.0);
            through = through && y == 0.0;
        }
        let bottom = if through {
            let mut all = top;
            all.adjoin(margin.bottom);
            top = all;
            all
        } else if bottom_adjoins {
            let mut bottom = pending;
            bottom.adjoin(margin.bottom);
            bottom
        } else {
            y += pending.resolve();
            Margins::of(margin.bottom)
        };
        if let Some(marker) = &block.marker {
            self.marker(block, marker, (content_x, content_width), &mut children);
        }
        Laid {
            fragment: BoxFragment {
                node: block.node,
                rect: Rect {
                    x: margin.left,
                    y: 0.0,
                    width,
                    height: content_y + y + padding.bottom + border.bottom,
                },
                background: reset.background_color,
                border,
                border_color: reset.border_color,
                children,
            },
            top,
            bottom,
            through,
        }
    }

    fn inline(
        &mut self,
        inline: &InlineContent,
        lines: &Lines,
        origin: (f32, f32),
        width: f32,
        align: TextAlign,
        children: &mut Vec<Fragment>,
    ) {
        for line in &lines.lines {
            let offset = match align {
                TextAlign::Right => (width - line.width).max(0.0),
                TextAlign::Center => ((width - line.width) / 2.0).max(0.0),
                TextAlign::Start | TextAlign::End | TextAlign::Left | TextAlign::Justify => 0.0,
            };
            for item in lines.items.get(line.items.clone()).unwrap_or_default() {
                let Some(source) = inline.items.get(item.tag as usize) else {
                    continue;
                };
                let glyphs = lines.glyphs.get(item.glyphs.clone()).unwrap_or_default();
                if glyphs.is_empty() || !source.style.inherited.visible {
                    continue;
                }
                let start = self.glyphs.len() as u32;
                self.glyphs.extend_from_slice(glyphs);
                let baseline = origin.1 + line.baseline - item.baseline_shift;
                children.push(Fragment::Text(self.text(
                    Some(source.node),
                    &source.style,
                    Rect {
                        x: origin.0 + offset + item.x,
                        y: baseline - item.ascent,
                        width: item.width,
                        height: item.ascent + item.descent,
                    },
                    baseline,
                    item.font,
                    item.size,
                    start..self.glyphs.len() as u32,
                )));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &self,
        node: Option<NodeId>,
        style: &ComputedStyle,
        rect: Rect,
        baseline: f32,
        font: FontId,
        size: f32,
        glyphs: Range<u32>,
    ) -> TextFragment {
        TextFragment {
            node,
            rect,
            baseline,
            font,
            size,
            color: style.inherited.color,
            background: style.reset.background_color,
            decorations: style.inherited.decorations,
            glyphs,
        }
    }

    fn marker(
        &mut self,
        block: &BlockBox,
        marker: &InlineContent,
        content: (f32, f32),
        children: &mut Vec<Fragment>,
    ) {
        let lines = marker.shaped.lines(
            self.fonts,
            f32::INFINITY,
            run_style(&block.style, 0, 0.0),
            false,
        );
        let Some(line) = lines.lines.first() else {
            return;
        };
        let baseline = first_baseline(children).unwrap_or(line.baseline);
        let gap = block.style.inherited.font_size * 0.5;
        let style = &block.style;
        for item in lines.items.get(line.items.clone()).unwrap_or_default() {
            let glyphs = lines.glyphs.get(item.glyphs.clone()).unwrap_or_default();
            let start = self.glyphs.len() as u32;
            self.glyphs.extend_from_slice(glyphs);
            let x = if is_rtl(style) {
                content.0 + content.1 + gap + item.x
            } else {
                content.0 + item.x - line.width - gap
            };
            let fragment = self.text(
                block.node,
                style,
                Rect {
                    x,
                    y: baseline - item.ascent,
                    width: item.width,
                    height: item.ascent + item.descent,
                },
                baseline,
                item.font,
                item.size,
                start..self.glyphs.len() as u32,
            );
            children.push(Fragment::Text(TextFragment {
                decorations: TextDecorations::default(),
                background: Color::TRANSPARENT,
                ..fragment
            }));
        }
    }
}

fn first_baseline(children: &[Fragment]) -> Option<f32> {
    children.iter().find_map(|child| match child {
        Fragment::Text(text) => Some(text.baseline),
        Fragment::Box(fragment) => {
            first_baseline(&fragment.children).map(|baseline| baseline + fragment.rect.y)
        }
    })
}
