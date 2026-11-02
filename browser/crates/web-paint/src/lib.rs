#[cfg(feature = "raster")]
mod raster;

use std::ops::Range;

use linebender_resource_handle::FontData;
use web_dom::NodeId;
use web_layout::{BoxFragment, Fragment, FragmentTree, TextFragment};
use web_style::Color;
use web_text::{Fonts, Glyph};

pub use web_layout::Rect;
pub use web_style::Color;

#[derive(Clone, PartialEq, Debug)]
pub enum Item {
    Rect {
        rect: Rect,
        color: Color,
    },
    Glyphs {
        rect: Rect,
        node: Option<NodeId>,
        font: u32,
        size: f32,
        color: Color,
        glyphs: Range<u32>,
    },
}

#[derive(Clone, Debug)]
pub struct DisplayList {
    pub width: f32,
    pub height: f32,
    pub background: Color,
    pub items: Vec<Item>,
    pub glyphs: Vec<Glyph>,
    pub fonts: Vec<FontData>,
}

impl DisplayList {
    pub fn build(tree: &FragmentTree, fonts: &Fonts, width: f32) -> Self {
        let mut builder = Builder {
            tree,
            items: Vec::new(),
            glyphs: Vec::new(),
            fonts: Vec::new(),
            font_ids: Vec::new(),
            source: fonts,
        };
        builder.box_fragment(&tree.root, 0.0, 0.0);
        DisplayList {
            width,
            height: tree.height(),
            background: Color::WHITE,
            items: builder.items,
            glyphs: builder.glyphs,
            fonts: builder.fonts,
        }
    }

    pub fn glyphs(&self, range: &Range<u32>) -> &[Glyph] {
        self.glyphs
            .get(range.start as usize..range.end as usize)
            .unwrap_or_default()
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        self.items.iter().rev().find_map(|item| match item {
            Item::Glyphs {
                rect,
                node: Some(node),
                ..
            } if x >= rect.x
                && y >= rect.y
                && x < rect.x + rect.width
                && y < rect.y + rect.height =>
            {
                Some(*node)
            }
            _ => None,
        })
    }

    pub fn heap_bytes(&self) -> usize {
        self.items.capacity() * size_of::<Item>()
            + self.glyphs.capacity() * size_of::<Glyph>()
            + self.fonts.capacity() * size_of::<FontData>()
    }

    #[cfg(feature = "raster")]
    pub fn rasterize(&self, scroll_y: f32, width: u16, height: u16) -> Vec<u8> {
        raster::rasterize(self, scroll_y, width, height)
    }
}

struct Builder<'a> {
    tree: &'a FragmentTree,
    items: Vec<Item>,
    glyphs: Vec<Glyph>,
    fonts: Vec<FontData>,
    font_ids: Vec<(web_text::FontId, u32)>,
    source: &'a Fonts,
}

impl Builder<'_> {
    fn box_fragment(&mut self, fragment: &BoxFragment, x: f32, y: f32) {
        let rect = Rect {
            x: x + fragment.rect.x,
            y: y + fragment.rect.y,
            ..fragment.rect
        };
        self.fill(rect, fragment.background);
        let border = fragment.border;
        let color = fragment.border_color;
        self.fill(
            Rect {
                height: border.top,
                ..rect
            },
            color,
        );
        self.fill(
            Rect {
                y: rect.y + rect.height - border.bottom,
                height: border.bottom,
                ..rect
            },
            color,
        );
        self.fill(
            Rect {
                width: border.left,
                ..rect
            },
            color,
        );
        self.fill(
            Rect {
                x: rect.x + rect.width - border.right,
                width: border.right,
                ..rect
            },
            color,
        );
        for child in &fragment.children {
            match child {
                Fragment::Box(child) => self.box_fragment(child, rect.x, rect.y),
                Fragment::Text(text) => self.text(text, rect.x, rect.y),
            }
        }
    }

    fn fill(&mut self, rect: Rect, color: Color) {
        if color.a != 0 && rect.width > 0.0 && rect.height > 0.0 {
            self.items.push(Item::Rect { rect, color });
        }
    }

    fn text(&mut self, text: &TextFragment, x: f32, y: f32) {
        let rect = Rect {
            x: x + text.rect.x,
            y: y + text.rect.y,
            ..text.rect
        };
        self.fill(rect, text.background);
        let Some(font) = self.font(text.font) else {
            return;
        };
        let baseline = y + text.baseline;
        let source = self
            .tree
            .glyphs
            .get(text.glyphs.start as usize..text.glyphs.end as usize)
            .unwrap_or_default();
        let start = self.glyphs.len() as u32;
        self.glyphs.extend(source.iter().map(|glyph| Glyph {
            id: glyph.id,
            x: rect.x + glyph.x,
            y: baseline + glyph.y,
        }));
        self.items.push(Item::Glyphs {
            rect,
            node: text.node,
            font,
            size: text.size,
            color: text.color,
            glyphs: start..self.glyphs.len() as u32,
        });
        let decorations = [
            (text.decorations.underline, text.underline),
            (text.decorations.line_through, text.strikeout),
        ];
        for (color, (offset, thickness)) in decorations {
            if let Some(color) = color {
                self.fill(
                    Rect {
                        x: rect.x,
                        y: baseline - offset,
                        width: rect.width,
                        height: thickness,
                    },
                    color,
                );
            }
        }
    }

    fn font(&mut self, font: web_text::FontId) -> Option<u32> {
        if let Some(&(_, index)) = self.font_ids.iter().find(|&&(id, _)| id == font) {
            return Some(index);
        }
        let data = self.source.data(font)?.clone();
        let index = self.fonts.len() as u32;
        self.fonts.push(data);
        self.font_ids.push((font, index));
        Some(index)
    }
}
