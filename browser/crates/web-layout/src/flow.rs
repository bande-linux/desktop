use std::ops::Range;

use web_dom::NodeId;
use web_style::{
    Clear, Color, ComputedStyle, Float, LengthPercentage, Position, Sides, TextAlign,
    TextDecorations,
};
use web_text::{AtomicSize, FontId, Fonts, Glyph, Lines};

use crate::box_tree::{
    BlockBox, BoxTree, Contents, InlineContent, InlineFrame, InlineItem, ItemKind, is_rtl,
    run_style,
};
use crate::fragment::{BoxFragment, Extra, Fragment, FragmentTree, Layer, Rect, TextFragment};

pub(crate) type ImageSizes<'a> = &'a dyn Fn(&str) -> Option<(f32, f32)>;

struct Extent {
    tag: u32,
    left: f32,
    right: f32,
    start: bool,
    end: bool,
    depth: usize,
    shift: f32,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Margins {
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

pub(crate) struct Laid {
    pub(crate) fragment: BoxFragment,
    top: Margins,
    bottom: Margins,
    through: bool,
    pub(crate) baseline: Option<f32>,
    pub(crate) margin: Sides,
}

impl Laid {
    pub(crate) fn outer(&self) -> (f32, f32) {
        (
            self.fragment.rect.width + self.margin.left + self.margin.right,
            self.fragment.rect.height + self.margin.top + self.margin.bottom,
        )
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Sizing {
    pub(crate) width: Option<f32>,
    pub(crate) height: Option<f32>,
    pub(crate) shrink: bool,
    pub(crate) root: bool,
}

#[derive(Clone, Copy)]
struct Placed {
    rect: Rect,
    left: bool,
}

#[derive(Default)]
pub(crate) struct Bfc {
    floats: Vec<Placed>,
}

impl Bfc {
    fn space(&self, top: f32, height: f32, left: f32, right: f32) -> (f32, f32) {
        let bottom = top + height.max(1.0);
        let mut space = (left, right);
        for float in &self.floats {
            let rect = float.rect;
            if rect.y < bottom && rect.y + rect.height > top {
                if float.left {
                    space.0 = space.0.max(rect.x + rect.width);
                } else {
                    space.1 = space.1.min(rect.x);
                }
            }
        }
        space
    }

    fn place(&mut self, size: (f32, f32), left: bool, top: f32, edges: (f32, f32)) -> (f32, f32) {
        let mut y = self
            .floats
            .iter()
            .map(|float| float.rect.y)
            .fold(top, f32::max);
        for _ in 0..=self.floats.len() {
            let (start, end) = self.space(y, size.1, edges.0, edges.1);
            if end - start >= size.0 || (start == edges.0 && end == edges.1) {
                let x = if left { start } else { end - size.0 };
                self.floats.push(Placed {
                    rect: Rect {
                        x,
                        y,
                        width: size.0,
                        height: size.1,
                    },
                    left,
                });
                return (x, y);
            }
            y = self
                .floats
                .iter()
                .map(|float| float.rect.y + float.rect.height)
                .filter(|&bottom| bottom > y)
                .fold(f32::INFINITY, f32::min);
            if !y.is_finite() {
                break;
            }
        }
        let x = if left { edges.0 } else { edges.1 - size.0 };
        (x, y.max(top))
    }

    fn clearance(&self, clear: Clear) -> f32 {
        self.floats
            .iter()
            .filter(|float| match clear {
                Clear::None => false,
                Clear::Left => float.left,
                Clear::Right => !float.left,
                Clear::Both => true,
            })
            .map(|float| float.rect.y + float.rect.height)
            .fold(f32::NEG_INFINITY, f32::max)
    }

    fn bottom(&self) -> f32 {
        self.floats
            .iter()
            .map(|float| float.rect.y + float.rect.height)
            .fold(f32::NEG_INFINITY, f32::max)
    }
}

struct Absolute<'b> {
    block: &'b BlockBox,
    x: f32,
    y: f32,
}

pub(crate) struct Flow<'a, 'b> {
    pub(crate) fonts: &'a mut Fonts,
    pub(crate) glyphs: Vec<Glyph>,
    pub(crate) images: ImageSizes<'a>,
    viewport: (f32, f32),
    absolutes: Vec<Absolute<'b>>,
}

impl BoxTree {
    pub fn layout(
        &self,
        viewport: (f32, f32),
        fonts: &mut Fonts,
        images: &dyn Fn(&str) -> Option<(f32, f32)>,
    ) -> FragmentTree {
        let mut flow = Flow {
            fonts,
            glyphs: Vec::new(),
            images,
            viewport,
            absolutes: Vec::new(),
        };
        let root = match &self.root {
            Some(root) => {
                let mut bfc = Bfc::default();
                let laid = flow.block(
                    root,
                    (viewport.0.max(0.0), Some(viewport.1)),
                    (0.0, 0.0),
                    &mut bfc,
                    Sizing {
                        root: true,
                        ..Sizing::default()
                    },
                );
                let mut fragment = laid.fragment;
                fragment.rect.y = laid.top.resolve();
                fragment
            }
            None => BoxFragment {
                node: None,
                rect: Rect::default(),
                background: Color::TRANSPARENT,
                extra: None,
                children: Vec::new(),
            },
        };
        FragmentTree {
            root,
            glyphs: flow.glyphs,
        }
    }
}

pub(crate) fn establishes_bfc(style: &ComputedStyle) -> bool {
    style.reset.float != Float::None
        || style.is_out_of_flow()
        || style.reset.clips
        || matches!(
            style.display(),
            web_style::Display::InlineBlock
                | web_style::Display::Flex
                | web_style::Display::InlineFlex
                | web_style::Display::Grid
                | web_style::Display::InlineGrid
                | web_style::Display::Table
                | web_style::Display::InlineTable
                | web_style::Display::TableCell
                | web_style::Display::TableCaption
        )
}

fn extra(
    border: Sides,
    border_color: Sides<Color>,
    layer: Layer,
    image: Option<std::sync::Arc<str>>,
    background_image: Option<std::sync::Arc<str>>,
) -> Option<Box<Extra>> {
    let bordered = border != Sides::default();
    (bordered || layer != Layer::FLOW || image.is_some() || background_image.is_some()).then(|| {
        Box::new(Extra {
            border,
            border_color,
            padding: Sides::default(),
            background: web_style::BackgroundLayout::default(),
            layer,
            image,
            background_image,
        })
    })
}

fn layer(style: &ComputedStyle) -> Layer {
    let reset = &style.reset;
    let positioned = style.is_positioned();
    Layer {
        z: positioned.then(|| reset.z_index.unwrap_or(0)),
        stacking: (positioned && reset.z_index.is_some()) || reset.opacity < 1.0,
        clips: reset.clips,
        opacity: reset.opacity,
    }
}

pub(crate) fn resolve_margins(style: &ComputedStyle, width: f32) -> Sides<Option<f32>> {
    style
        .reset
        .margin
        .map(|side| side.map(|side| side.resolve(width)))
}

fn relative_offset(style: &ComputedStyle, cb: (f32, Option<f32>)) -> (f32, f32) {
    if !matches!(style.reset.position, Position::Relative | Position::Sticky) {
        return (0.0, 0.0);
    }
    let inset = style.reset.inset;
    let x = match (inset.left, inset.right) {
        (Some(left), _) => left.resolve(cb.0),
        (None, Some(right)) => -right.resolve(cb.0),
        _ => 0.0,
    };
    let height = cb.1.unwrap_or(0.0);
    let y = match (inset.top, inset.bottom) {
        (Some(top), _) => top.resolve(height),
        (None, Some(bottom)) => -bottom.resolve(height),
        _ => 0.0,
    };
    (x, y)
}

fn resolve_height(value: LengthPercentage, cb: Option<f32>) -> Option<f32> {
    match cb {
        Some(height) => Some(value.resolve(height)),
        None if value.percent == 0.0 => Some(value.px),
        None => None,
    }
}

impl<'a, 'b> Flow<'a, 'b> {
    pub(crate) fn mark(&self) -> (usize, usize) {
        (self.glyphs.len(), self.absolutes.len())
    }

    pub(crate) fn rewind(&mut self, mark: (usize, usize)) {
        self.glyphs.truncate(mark.0);
        self.absolutes.truncate(mark.1);
    }

    pub(crate) fn register_absolute(&mut self, block: &'b BlockBox, origin: (f32, f32)) {
        self.absolutes.push(Absolute {
            block,
            x: origin.0,
            y: origin.1,
        });
    }

    pub(crate) fn replaced_size(&self, block: &BlockBox, cb: (f32, Option<f32>)) -> (f32, f32) {
        let reset = &block.style.reset;
        let natural = match &block.contents {
            Contents::Replaced(Some(source)) => (self.images)(source),
            _ => None,
        };
        let frame = frame(&block.style, cb.0);
        let border_box = |value: f32, frame: f32| {
            if reset.border_box {
                (value - frame).max(0.0)
            } else {
                value
            }
        };
        let width = reset
            .width
            .map(|width| border_box(width.resolve(cb.0), frame.0));
        let height = reset
            .height
            .and_then(|height| resolve_height(height, cb.1))
            .map(|height| border_box(height, frame.1));
        let ratio = natural
            .filter(|(w, h)| *w > 0.0 && *h > 0.0)
            .map(|(w, h)| h / w);
        let (mut width, mut height) = match (width, height, natural) {
            (Some(width), Some(height), _) => (width, height),
            (Some(width), None, _) => (width, ratio.map_or(150.0 * 0.5, |ratio| width * ratio)),
            (None, Some(height), _) => (ratio.map_or(300.0, |ratio| height / ratio), height),
            (None, None, Some(natural)) => natural,
            (None, None, None) => (0.0, 0.0),
        };
        if let Some(maximum) = reset
            .max_width
            .map(|max| border_box(max.resolve(cb.0), frame.0))
            && width > maximum
        {
            if let Some(ratio) = ratio
                && reset.height.is_none()
            {
                height = maximum * ratio;
            }
            width = maximum;
        }
        (width.max(0.0), height.max(0.0))
    }

    pub(crate) fn block(
        &mut self,
        block: &'b BlockBox,
        cb: (f32, Option<f32>),
        origin: (f32, f32),
        bfc: &mut Bfc,
        sizing: Sizing,
    ) -> Laid {
        let style = &block.style;
        let reset = &style.reset;
        let border = reset.border;
        let padding = reset.padding.map(|side| side.resolve(cb.0).max(0.0));
        let margins = resolve_margins(style, cb.0);
        let (mut margin_left, mut margin_right) = (margins.left, margins.right);
        let frame = border.left + border.right + padding.left + padding.right;
        let vertical_frame = border.top + border.bottom + padding.top + padding.bottom;
        let replaced = matches!(block.contents, Contents::Replaced(_));
        let replaced_size = replaced.then(|| self.replaced_size(block, cb));
        let specified = reset.width.map(|width| {
            let width = width.resolve(cb.0);
            if reset.border_box {
                width - frame
            } else {
                width
            }
        });
        let maximum = reset.max_width.map(|width| {
            let width = width.resolve(cb.0);
            if reset.border_box {
                width - frame
            } else {
                width
            }
        });
        let minimum = reset.min_width.resolve(cb.0);
        let available =
            (cb.0 - margin_left.unwrap_or(0.0) - margin_right.unwrap_or(0.0) - frame).max(0.0);
        let mut content_width = match (sizing.width, replaced_size, specified) {
            (Some(width), _, _) => (width - frame).max(0.0),
            (None, Some((width, _)), _) => width,
            (None, None, Some(width)) => width,
            (None, None, None) if sizing.shrink || matches!(block.contents, Contents::Table(_)) => {
                let (min, max) = self.intrinsic(block, cb);
                max.min(available).max(min)
            }
            (None, None, None) => available,
        };
        if sizing.width.is_none() && !replaced {
            if let Some(maximum) = maximum {
                content_width = content_width.min(maximum);
            }
            content_width = content_width.max(minimum);
        }
        content_width = content_width.max(0.0);
        if !sizing.shrink && sizing.width.is_none() {
            let rest = cb.0 - content_width - frame;
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
        let margin = Sides {
            top: margins.top.unwrap_or(0.0),
            right: margin_right.unwrap_or(0.0),
            bottom: margins.bottom.unwrap_or(0.0),
            left: margin_left.unwrap_or(0.0),
        };
        let width = content_width + frame;
        let content_x = border.left + padding.left;
        let content_y = border.top + padding.top;
        let border_origin = (origin.0 + margin.left, origin.1);
        let content_origin = (border_origin.0 + content_x, border_origin.1 + content_y);
        let new_bfc = sizing.root || establishes_bfc(style);
        let mut own_bfc = Bfc::default();
        let bfc: &mut Bfc = if new_bfc { &mut own_bfc } else { bfc };
        let positioned = sizing.root || style.is_positioned();
        let mark = self.absolutes.len();
        let specified_height = sizing
            .height
            .map(|height| (height - vertical_frame).max(0.0))
            .or_else(|| {
                reset
                    .height
                    .and_then(|height| resolve_height(height, cb.1))
                    .map(|height| {
                        if reset.border_box {
                            (height - vertical_frame).max(0.0)
                        } else {
                            height
                        }
                    })
            });
        let children_height = specified_height;
        let top_adjoins = !new_bfc && border.top == 0.0 && padding.top == 0.0;
        let bottom_adjoins =
            !new_bfc && border.bottom == 0.0 && padding.bottom == 0.0 && specified_height.is_none();
        let mut top = Margins::of(margin.top);
        let mut children = Vec::new();
        let mut y = 0.0;
        let mut through = top_adjoins && bottom_adjoins && !replaced;
        let mut pending = Margins::default();
        let mut baseline = None;
        match &block.contents {
            Contents::Blocks(blocks) => {
                let mut at_top = top_adjoins;
                for child in blocks {
                    let child_style = &child.style;
                    let static_y = content_origin.1 + y + pending.resolve();
                    if child_style.is_out_of_flow() {
                        self.absolutes.push(Absolute {
                            block: child,
                            x: content_origin.0,
                            y: static_y,
                        });
                        continue;
                    }
                    if child_style.reset.float != Float::None {
                        let fragment = self.float(
                            child,
                            (content_width, children_height),
                            static_y,
                            bfc,
                            (content_origin.0, content_origin.0 + content_width),
                            border_origin,
                        );
                        through = false;
                        children.push(Fragment::Box(fragment));
                        continue;
                    }
                    let child_margin_top = resolve_margins(child_style, content_width)
                        .top
                        .unwrap_or(0.0);
                    let mut estimate = if at_top {
                        content_origin.1
                    } else {
                        let mut collapsed = pending;
                        collapsed.adjoin_all(leading_margins(child, content_width));
                        content_origin.1 + y + collapsed.resolve()
                    };
                    if child_style.reset.clear != Clear::None {
                        let floor = bfc.clearance(child_style.reset.clear);
                        if floor > estimate {
                            y = floor - content_origin.1 - child_margin_top;
                            estimate = floor;
                            at_top = false;
                            through = false;
                            pending = Margins::default();
                        }
                    }
                    let (space_width, shift) = if establishes_bfc(child_style) {
                        let (left, right) = bfc.space(
                            estimate,
                            1.0,
                            content_origin.0,
                            content_origin.0 + content_width,
                        );
                        ((right - left).max(0.0), left - content_origin.0)
                    } else {
                        (content_width, 0.0)
                    };
                    let laid = self.block(
                        child,
                        (space_width, children_height),
                        (content_origin.0 + shift, estimate),
                        bfc,
                        Sizing::default(),
                    );
                    if at_top {
                        top.adjoin_all(laid.top);
                        if laid.through {
                            top.adjoin_all(laid.bottom);
                            keep_empty(
                                laid,
                                (content_x + shift, content_y + estimate - content_origin.1),
                                &mut children,
                            );
                            continue;
                        }
                    } else {
                        pending.adjoin_all(laid.top);
                        if laid.through {
                            pending.adjoin_all(laid.bottom);
                            keep_empty(
                                laid,
                                (content_x + shift, content_y + estimate - content_origin.1),
                                &mut children,
                            );
                            continue;
                        }
                        y += pending.resolve();
                    }
                    at_top = false;
                    through = false;
                    let (dx, dy) = relative_offset(child_style, (content_width, children_height));
                    let mut fragment = laid.fragment;
                    fragment.rect.x += content_x + shift + dx;
                    fragment.rect.y = content_y + y + dy;
                    if let Some(child_baseline) = laid.baseline {
                        baseline = Some(content_y + y + child_baseline);
                    }
                    y += fragment.rect.height;
                    pending = laid.bottom;
                    children.push(Fragment::Box(fragment));
                }
            }
            Contents::Inline(inline) => {
                let (height, last) = self.inline(
                    block,
                    inline,
                    content_width,
                    content_origin,
                    (content_x, content_y),
                    border_origin,
                    bfc,
                    &mut children,
                );
                if height > 0.0 || !children.is_empty() {
                    through = false;
                    y = height;
                    baseline = last.map(|last| content_y + last);
                }
            }
            Contents::Grid(items) => {
                y = self.grid(
                    block,
                    items,
                    (content_width, specified_height),
                    content_origin,
                    (content_x, content_y),
                    &mut children,
                );
                through = through && y == 0.0 && children.is_empty();
            }
            Contents::Flex(items) => {
                y = self.flex(
                    block,
                    items,
                    (content_width, specified_height),
                    content_origin,
                    (content_x, content_y),
                    &mut children,
                );
                through = through && y == 0.0 && children.is_empty();
            }
            Contents::Replaced(_) => {
                y = replaced_size.map_or(0.0, |(_, height)| height);
                through = false;
            }
            Contents::Table(table) => {
                y = self.table(
                    block,
                    table,
                    content_width,
                    content_origin,
                    (content_x, content_y),
                    &mut children,
                );
                through = false;
            }
        }
        if let Some(height) = specified_height {
            y = height;
            through = through && y == 0.0;
        } else {
            if new_bfc && !sizing.root {
                y = y.max(bfc.bottom() - content_origin.1);
            }
            y = y.max(reset.min_height.resolve(cb.1.unwrap_or(0.0)));
            if let Some(maximum) = reset.max_height.and_then(|max| resolve_height(max, cb.1)) {
                y = y.min(maximum);
            }
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
            if specified_height.is_none() {
                y += pending.resolve();
            }
            Margins::of(margin.bottom)
        };
        if let Some(marker) = block.marker.as_ref().filter(|_| style.inherited.visible) {
            self.marker(block, marker, (content_x, content_width), &mut children);
        }
        let height = content_y + y + padding.bottom + border.bottom;
        if positioned {
            let padding_box = (
                border_origin.0 + border.left,
                border_origin.1 + border.top,
                width - border.left - border.right,
                height - border.top - border.bottom,
            );
            let pending: Vec<Absolute<'b>> = if sizing.root {
                self.absolutes
                    .drain(mark.min(self.absolutes.len())..)
                    .collect()
            } else {
                let mut kept = Vec::new();
                let mut taken = Vec::new();
                for absolute in self.absolutes.drain(mark.min(self.absolutes.len())..) {
                    if absolute.block.style.reset.position == Position::Fixed {
                        kept.push(absolute);
                    } else {
                        taken.push(absolute);
                    }
                }
                self.absolutes.extend(kept);
                taken
            };
            for absolute in pending {
                let fixed = absolute.block.style.reset.position == Position::Fixed;
                let containing = if fixed || sizing.root {
                    (0.0, 0.0, self.viewport.0, self.viewport.1)
                } else {
                    padding_box
                };
                let mut fragment = self.absolute(&absolute, containing);
                fragment.rect.x -= border_origin.0;
                fragment.rect.y -= border_origin.1;
                children.push(Fragment::Box(fragment));
            }
        }
        Laid {
            fragment: BoxFragment {
                node: block.node,
                rect: Rect {
                    x: margin.left,
                    y: 0.0,
                    width,
                    height,
                },
                background: reset.background_color,
                extra: extra(
                    border,
                    reset.border_color,
                    layer(style),
                    match &block.contents {
                        Contents::Replaced(source) => source.clone(),
                        _ => None,
                    },
                    reset.background_image.clone(),
                )
                .map(|mut extra| {
                    extra.padding = padding;
                    extra.background = reset.background;
                    extra
                }),
                children,
            },
            top,
            bottom,
            through,
            baseline,
            margin,
        }
    }

    fn float(
        &mut self,
        child: &'b BlockBox,
        cb: (f32, Option<f32>),
        top: f32,
        bfc: &mut Bfc,
        edges: (f32, f32),
        parent_origin: (f32, f32),
    ) -> BoxFragment {
        let laid = self.block(
            child,
            cb,
            (edges.0, top),
            &mut Bfc::default(),
            Sizing {
                shrink: true,
                ..Sizing::default()
            },
        );
        let size = laid.outer();
        let left = child.style.reset.float == Float::Left;
        let (x, y) = bfc.place(size, left, top, edges);
        let (dx, dy) = relative_offset(&child.style, cb);
        let mut fragment = laid.fragment;
        fragment.rect.x = x + laid.margin.left - parent_origin.0 + dx;
        fragment.rect.y = y + laid.margin.top - parent_origin.1 + dy;
        fragment
    }

    fn absolute(&mut self, absolute: &Absolute<'b>, cb: (f32, f32, f32, f32)) -> BoxFragment {
        let style = &absolute.block.style;
        let inset = style.reset.inset;
        let (cb_width, cb_height) = (cb.2, cb.3);
        let left = inset.left.map(|left| left.resolve(cb_width));
        let right = inset.right.map(|right| right.resolve(cb_width));
        let top = inset.top.map(|top| top.resolve(cb_height));
        let bottom = inset.bottom.map(|bottom| bottom.resolve(cb_height));
        let margins = resolve_margins(style, cb_width).map(|side| side.unwrap_or(0.0));
        let stretch_width = match (left, right, style.reset.width) {
            (Some(left), Some(right), None) => {
                Some((cb_width - left - right - margins.left - margins.right).max(0.0))
            }
            _ => None,
        };
        let stretch_height = match (top, bottom, style.reset.height) {
            (Some(top), Some(bottom), None) => {
                Some((cb_height - top - bottom - margins.top - margins.bottom).max(0.0))
            }
            _ => None,
        };
        let available = cb_width - left.unwrap_or(0.0) - right.unwrap_or(0.0);
        let laid = self.block(
            absolute.block,
            (available.max(0.0), Some(cb_height)),
            (absolute.x, absolute.y),
            &mut Bfc::default(),
            Sizing {
                width: stretch_width,
                height: stretch_height,
                shrink: true,
                root: false,
            },
        );
        let (outer_width, outer_height) = laid.outer();
        let x = match (left, right) {
            (Some(left), _) => cb.0 + left,
            (None, Some(right)) => cb.0 + cb_width - right - outer_width,
            (None, None) => absolute.x,
        };
        let y = match (top, bottom) {
            (Some(top), _) => cb.1 + top,
            (None, Some(bottom)) => cb.1 + cb_height - bottom - outer_height,
            (None, None) => absolute.y,
        };
        let auto = style.reset.margin;
        let solve = |start: Option<f32>,
                     end: Option<f32>,
                     size: f32,
                     extent: f32,
                     margins: (
            Option<web_style::LengthPercentage>,
            Option<web_style::LengthPercentage>,
        ),
                     resolved: (f32, f32)| {
            match (start, end) {
                (Some(start), Some(end)) => {
                    let free = extent - start - end - size;
                    match margins {
                        (None, None) => Some(free / 2.0),
                        (None, Some(_)) => Some(free - resolved.1),
                        _ => None,
                    }
                }
                _ => None,
            }
        };
        let margin_left = solve(
            left,
            right,
            laid.fragment.rect.width,
            cb_width,
            (auto.left, auto.right),
            (laid.margin.left, laid.margin.right),
        )
        .unwrap_or(laid.margin.left);
        let margin_top = solve(
            top,
            bottom,
            laid.fragment.rect.height,
            cb_height,
            (auto.top, auto.bottom),
            (laid.margin.top, laid.margin.bottom),
        )
        .unwrap_or(laid.margin.top);
        let mut fragment = laid.fragment;
        fragment.rect.x = x + margin_left;
        fragment.rect.y = y + margin_top;
        fragment
    }

    #[allow(clippy::too_many_arguments)]
    fn inline(
        &mut self,
        block: &'b BlockBox,
        inline: &'b InlineContent,
        width: f32,
        origin: (f32, f32),
        offset: (f32, f32),
        border_origin: (f32, f32),
        bfc: &mut Bfc,
        children: &mut Vec<Fragment>,
    ) -> (f32, Option<f32>) {
        for child in &inline.out_of_flow {
            if child.style.is_out_of_flow() {
                self.absolutes.push(Absolute {
                    block: child,
                    x: origin.0,
                    y: origin.1,
                });
            } else {
                let fragment = self.float(
                    child,
                    (width, None),
                    origin.1,
                    bfc,
                    (origin.0, origin.0 + width),
                    border_origin,
                );
                children.push(Fragment::Box(fragment));
            }
        }
        let mut atomics: Vec<(Laid, AtomicSize)> = Vec::new();
        for atomic in &inline.atomics {
            let laid = self.block(
                atomic,
                (width, None),
                origin,
                &mut Bfc::default(),
                Sizing {
                    shrink: true,
                    ..Sizing::default()
                },
            );
            let (outer_width, outer_height) = laid.outer();
            let ascent = match (&atomic.contents, laid.baseline) {
                (Contents::Replaced(_), _) | (_, None) => outer_height,
                (_, Some(baseline)) => (laid.margin.top + baseline).min(outer_height),
            };
            let size = AtomicSize {
                width: outer_width,
                ascent,
                descent: outer_height - ascent,
            };
            atomics.push((laid, size));
        }
        let sizes: Vec<AtomicSize> = atomics.iter().map(|(_, size)| *size).collect();
        let strut = run_style(self.fonts, &block.style, u32::MAX, 0.0);
        let line_estimate = strut.line_height.unwrap_or(strut.size * 1.2);
        let lines = inline.shaped.lines(
            self.fonts,
            &mut |top| {
                let (left, right) =
                    bfc.space(origin.1 + top, line_estimate, origin.0, origin.0 + width);
                (left - origin.0, (right - left).max(0.0))
            },
            strut,
            block.style.inherited.text_align == TextAlign::Justify,
            &sizes,
        );
        if lines.lines.is_empty() {
            return (0.0, None);
        }
        let rtl = is_rtl(&block.style);
        let align = match block.style.inherited.text_align {
            TextAlign::Start | TextAlign::Justify if rtl => TextAlign::Right,
            TextAlign::End if !rtl => TextAlign::Right,
            TextAlign::End => TextAlign::Left,
            align => align,
        };
        let mut atomics: Vec<Option<(Laid, AtomicSize)>> = atomics.into_iter().map(Some).collect();
        self.lines(inline, &lines, offset, align, &mut atomics, children);
        let last = lines.lines.last().map(|line| line.baseline);
        (lines.height, last)
    }

    fn lines(
        &mut self,
        inline: &InlineContent,
        lines: &Lines,
        origin: (f32, f32),
        align: TextAlign,
        atomics: &mut [Option<(Laid, AtomicSize)>],
        children: &mut Vec<Fragment>,
    ) {
        for line in &lines.lines {
            let free = (line.available - line.width).max(0.0);
            let offset = line.left
                + match align {
                    TextAlign::Right => free,
                    TextAlign::Center => free / 2.0,
                    TextAlign::Start | TextAlign::End | TextAlign::Left | TextAlign::Justify => 0.0,
                };
            let items = lines.items.get(line.items.clone()).unwrap_or_default();
            let mut extents: Vec<Extent> = Vec::new();
            for item in items {
                let Some(source) = inline.items.get(item.tag as usize) else {
                    continue;
                };
                let x = origin.0 + offset + item.x;
                let (mut current, edge) = match source.kind {
                    ItemKind::Text | ItemKind::Atomic(_) => (source.parent, None),
                    ItemKind::Start => (Some(item.tag), Some((item.tag, true))),
                    ItemKind::End(start) => (Some(start), Some((start, false))),
                };
                while let Some(tag) = current {
                    let extent = match extents.iter().position(|extent| extent.tag == tag) {
                        Some(index) => index,
                        None => {
                            extents.push(Extent {
                                tag,
                                left: x,
                                right: x + item.width,
                                start: false,
                                end: false,
                                depth: 0,
                                shift: 0.0,
                            });
                            extents.len() - 1
                        }
                    };
                    if let Some(extent) = extents.get_mut(extent) {
                        extent.left = extent.left.min(x);
                        extent.right = extent.right.max(x + item.width);
                        match edge {
                            Some((edge, true)) if edge == tag => {
                                extent.start = true;
                                extent.shift = item.baseline_shift;
                            }
                            Some((edge, false)) if edge == tag => {
                                extent.end = true;
                                extent.shift = item.baseline_shift;
                            }
                            _ => {}
                        }
                    }
                    current = inline.items.get(tag as usize).and_then(|item| item.parent);
                }
            }
            for extent in &mut extents {
                extent.depth = std::iter::successors(Some(extent.tag), |&tag| {
                    inline.items.get(tag as usize).and_then(|item| item.parent)
                })
                .count();
            }
            extents.sort_by_key(|extent| extent.depth);
            for extent in &extents {
                if let Some(source) = inline.items.get(extent.tag as usize) {
                    children.push(Fragment::Box(self.inline_box(
                        source,
                        extent,
                        origin.1 + line.baseline,
                    )));
                }
            }
            for item in items {
                let Some(source) = inline.items.get(item.tag as usize) else {
                    continue;
                };
                let baseline = origin.1 + line.baseline - item.baseline_shift;
                let x = origin.0 + offset + item.x;
                if let ItemKind::Atomic(index) = source.kind {
                    if let Some((laid, size)) =
                        atomics.get_mut(usize::from(index)).and_then(Option::take)
                    {
                        let (dx, dy) = relative_offset(&source.style, (line.available, None));
                        let mut fragment = laid.fragment;
                        fragment.rect.x = x + laid.margin.left + dx;
                        fragment.rect.y = baseline - size.ascent + laid.margin.top + dy;
                        children.push(Fragment::Box(fragment));
                    }
                    continue;
                }
                let glyphs = lines.glyphs.get(item.glyphs.clone()).unwrap_or_default();
                if glyphs.is_empty() || !source.style.inherited.visible {
                    continue;
                }
                let start = self.glyphs.len() as u32;
                self.glyphs.extend_from_slice(glyphs);
                children.push(Fragment::Text(self.text(
                    Some(source.node),
                    &source.style,
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
                )));
            }
        }
    }

    fn inline_box(&mut self, source: &InlineItem, extent: &Extent, baseline: f32) -> BoxFragment {
        let style = &source.style;
        let frame = InlineFrame::of(style);
        let left = extent.left + if extent.start { frame.margin.0 } else { 0.0 };
        let right = extent.right - if extent.end { frame.margin.1 } else { 0.0 };
        let key = run_style(self.fonts, style, 0, 0.0).font;
        let (ascent, descent) = self
            .fonts
            .primary(key)
            .map(|font| {
                let metrics = self.fonts.metrics(font);
                let scale = style.inherited.font_size / metrics.units_per_em;
                (metrics.ascent * scale, metrics.descent * scale)
            })
            .unwrap_or((
                style.inherited.font_size * 0.8,
                style.inherited.font_size * 0.2,
            ));
        let reset = &style.reset;
        let shift = extent.shift;
        let top = baseline - shift - ascent - reset.padding.top.px - reset.border.top;
        let bottom = baseline - shift + descent + reset.padding.bottom.px + reset.border.bottom;
        let (start_border, end_border) = frame.border;
        let (left_border, right_border) = if style.is_rtl() {
            (end_border, start_border)
        } else {
            (start_border, end_border)
        };
        BoxFragment {
            node: Some(source.node),
            rect: Rect {
                x: left,
                y: top,
                width: (right - left).max(0.0),
                height: (bottom - top).max(0.0),
            },
            background: reset.background_color,
            extra: extra(
                Sides {
                    top: reset.border.top,
                    right: if extent.end { right_border } else { 0.0 },
                    bottom: reset.border.bottom,
                    left: if extent.start { left_border } else { 0.0 },
                },
                reset.border_color,
                Layer::FLOW,
                None,
                None,
            ),
            children: Vec::new(),
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
        let strut = run_style(self.fonts, &block.style, 0, 0.0);
        let lines =
            marker
                .shaped
                .lines(self.fonts, &mut |_| (0.0, f32::INFINITY), strut, false, &[]);
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
                ..fragment
            }));
        }
    }
}

fn leading_margins(block: &BlockBox, width: f32) -> Margins {
    let mut margins = Margins::default();
    let mut current = block;
    for _ in 0..64 {
        let style = &current.style;
        margins.adjoin(resolve_margins(style, width).top.unwrap_or(0.0));
        let reset = &style.reset;
        if establishes_bfc(style)
            || reset.border.top != 0.0
            || reset.padding.top.resolve(width) != 0.0
        {
            break;
        }
        let Contents::Blocks(children) = &current.contents else {
            break;
        };
        let Some(first) = children
            .iter()
            .find(|child| !child.style.is_out_of_flow() && child.style.reset.float == Float::None)
        else {
            break;
        };
        current = first;
    }
    margins
}

fn keep_empty(laid: Laid, at: (f32, f32), children: &mut Vec<Fragment>) {
    if laid.fragment.children.is_empty() {
        return;
    }
    let mut fragment = laid.fragment;
    fragment.rect.x += at.0;
    fragment.rect.y = at.1;
    children.push(Fragment::Box(fragment));
}

fn first_baseline(children: &[Fragment]) -> Option<f32> {
    children.iter().find_map(|child| match child {
        Fragment::Text(text) => Some(text.baseline),
        Fragment::Box(fragment) => {
            first_baseline(&fragment.children).map(|baseline| baseline + fragment.rect.y)
        }
    })
}

pub(crate) fn frame(style: &ComputedStyle, width: f32) -> (f32, f32) {
    let reset = &style.reset;
    let padding = reset.padding.map(|side| side.resolve(width).max(0.0));
    (
        reset.border.left + reset.border.right + padding.left + padding.right,
        reset.border.top + reset.border.bottom + padding.top + padding.bottom,
    )
}
