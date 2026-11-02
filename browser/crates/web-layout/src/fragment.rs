use std::ops::Range;
use std::sync::Arc;

use web_dom::NodeId;
use web_style::{Color, Sides, TextDecorations};
use web_text::{FontId, Glyph};

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct BoxFragment {
    pub node: Option<NodeId>,
    pub rect: Rect,
    pub background: Color,
    pub border: Sides,
    pub border_color: Sides<Color>,
    pub layer: Layer,
    pub image: Option<Arc<str>>,
    pub background_image: Option<Arc<str>>,
    pub children: Vec<Fragment>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Layer {
    pub z: Option<i32>,
    pub stacking: bool,
    pub clips: bool,
    pub opacity: f32,
}

impl Default for Layer {
    fn default() -> Self {
        Layer::FLOW
    }
}

impl Layer {
    pub const FLOW: Layer = Layer {
        z: None,
        stacking: false,
        clips: false,
        opacity: 1.0,
    };
}

#[derive(Clone, PartialEq, Debug)]
pub struct TextFragment {
    pub node: Option<NodeId>,
    pub rect: Rect,
    pub baseline: f32,
    pub font: FontId,
    pub size: f32,
    pub color: Color,
    pub decorations: TextDecorations,
    pub glyphs: Range<u32>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Fragment {
    Box(BoxFragment),
    Text(TextFragment),
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct FragmentTree {
    pub root: BoxFragment,
    pub glyphs: Vec<Glyph>,
}

impl FragmentTree {
    pub fn height(&self) -> f32 {
        extent(&self.root, 0.0)
    }

    pub fn heap_bytes(&self) -> usize {
        self.glyphs.capacity() * size_of::<Glyph>() + box_bytes(&self.root)
    }
}

fn extent(fragment: &BoxFragment, y: f32) -> f32 {
    let top = y + fragment.rect.y;
    let own = top + fragment.rect.height;
    if fragment.layer.clips {
        return own;
    }
    fragment
        .children
        .iter()
        .map(|child| match child {
            Fragment::Box(child) => extent(child, top),
            Fragment::Text(text) => top + text.rect.y + text.rect.height,
        })
        .fold(own, f32::max)
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
