#[cfg(feature = "raster")]
mod raster;

use std::ops::Range;
use std::sync::Arc;

use linebender_resource_handle::FontData;
use web_dom::NodeId;
use web_layout::{BoxFragment, Fragment, FragmentTree, TextFragment};
use web_text::{Fonts, Glyph};

pub use web_image::Image;
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
        baseline: f32,
        node: Option<NodeId>,
        font: u32,
        size: f32,
        color: Color,
        glyphs: Range<u32>,
    },
    Image {
        rect: Rect,
        clip: Rect,
        image: u32,
    },
    PushClip {
        rect: Rect,
    },
    PushOpacity {
        alpha: f32,
    },
    Pop,
}

pub type ImageSource<'a> = &'a dyn Fn(&str) -> Option<Arc<Image>>;

#[derive(Clone, Debug, Default)]
pub struct DisplayList {
    pub width: f32,
    pub height: f32,
    pub background: Color,
    pub items: Vec<Item>,
    pub glyphs: Vec<Glyph>,
    pub fonts: Vec<FontData>,
    pub images: Vec<Arc<Image>>,
}

impl DisplayList {
    pub fn build(
        tree: &FragmentTree,
        fonts: &Fonts,
        width: f32,
        images: &dyn Fn(&str) -> Option<Arc<Image>>,
    ) -> Self {
        let mut builder = Builder {
            tree,
            items: Vec::new(),
            glyphs: Vec::new(),
            fonts: Vec::new(),
            font_ids: Vec::new(),
            source: fonts,
            images,
            image_list: Vec::new(),
            image_ids: Vec::new(),
            clips: Vec::new(),
        };
        builder.context(&tree.root, 0.0, 0.0);
        DisplayList {
            width,
            height: tree.height(),
            background: Color::WHITE,
            items: builder.items,
            glyphs: builder.glyphs,
            fonts: builder.fonts,
            images: builder.image_list,
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
            + self.images.capacity() * size_of::<Arc<Image>>()
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
    images: ImageSource<'a>,
    image_list: Vec<Arc<Image>>,
    image_ids: Vec<(Arc<str>, u32)>,
    clips: Vec<Rect>,
}

struct Deferred<'f> {
    z: i32,
    fragment: &'f BoxFragment,
    x: f32,
    y: f32,
    clips: Vec<Rect>,
}

impl Builder<'_> {
    fn context(&mut self, fragment: &BoxFragment, x: f32, y: f32) {
        let mut deferred = Vec::new();
        self.box_fragment(fragment, x, y, &mut deferred);
        deferred.sort_by_key(|entry| entry.z);
        let saved = std::mem::take(&mut self.clips);
        for entry in deferred {
            for &rect in &entry.clips {
                self.items.push(Item::PushClip { rect });
            }
            self.clips = entry.clips.clone();
            self.context(entry.fragment, entry.x, entry.y);
            for _ in &entry.clips {
                self.items.push(Item::Pop);
            }
        }
        self.clips = saved;
    }

    fn image(&mut self, source: &Arc<str>) -> Option<u32> {
        if let Some(&(_, index)) = self.image_ids.iter().find(|(known, _)| known == source) {
            return Some(index);
        }
        let image = (self.images)(source)?;
        let index = self.image_list.len() as u32;
        self.image_list.push(image);
        self.image_ids.push((source.clone(), index));
        Some(index)
    }

    fn box_fragment<'f>(
        &mut self,
        fragment: &'f BoxFragment,
        x: f32,
        y: f32,
        deferred: &mut Vec<Deferred<'f>>,
    ) {
        let rect = Rect {
            x: x + fragment.rect.x,
            y: y + fragment.rect.y,
            ..fragment.rect
        };
        let layer = fragment.layer();
        let faded = layer.opacity < 1.0;
        if faded {
            self.items.push(Item::PushOpacity {
                alpha: layer.opacity,
            });
        }
        self.fill(rect, fragment.background);
        if let Some(source) = fragment.background_image()
            && let Some(image) = self.image(source)
            && let Some(data) = self.image_list.get(image as usize)
        {
            self.items.push(Item::Image {
                rect: Rect {
                    width: data.width as f32,
                    height: data.height as f32,
                    ..rect
                },
                clip: rect,
                image,
            });
        }
        let (border, colors) = fragment.border();
        let sides = [
            (
                Rect {
                    height: border.top,
                    ..rect
                },
                colors.top,
            ),
            (
                Rect {
                    y: rect.y + rect.height - border.bottom,
                    height: border.bottom,
                    ..rect
                },
                colors.bottom,
            ),
            (
                Rect {
                    width: border.left,
                    ..rect
                },
                colors.left,
            ),
            (
                Rect {
                    x: rect.x + rect.width - border.right,
                    width: border.right,
                    ..rect
                },
                colors.right,
            ),
        ];
        for (side, color) in sides {
            self.fill(side, color);
        }
        if let Some(source) = fragment.image()
            && let Some(image) = self.image(source)
        {
            let inner = Rect {
                x: rect.x + border.left,
                y: rect.y + border.top,
                width: (rect.width - border.left - border.right).max(0.0),
                height: (rect.height - border.top - border.bottom).max(0.0),
            };
            self.items.push(Item::Image {
                rect: inner,
                clip: inner,
                image,
            });
        }
        if layer.clips {
            self.items.push(Item::PushClip { rect });
            self.clips.push(rect);
        }
        for child in &fragment.children {
            match child {
                Fragment::Box(child) => match child.layer().z {
                    Some(z) => deferred.push(Deferred {
                        z,
                        fragment: child,
                        x: rect.x,
                        y: rect.y,
                        clips: self.clips.clone(),
                    }),
                    None => self.box_fragment(child, rect.x, rect.y, deferred),
                },
                Fragment::Text(text) => self.text(text, rect.x, rect.y),
            }
        }
        if layer.clips {
            self.items.push(Item::Pop);
            self.clips.pop();
        }
        if faded {
            self.items.push(Item::Pop);
        }
    }

    fn fill(&mut self, rect: Rect, color: Color) {
        let rect = snap(rect);
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
        self.glyphs.extend(
            source
                .iter()
                .map(|glyph| Glyph::new(glyph.id, rect.x + glyph.x, glyph.y())),
        );
        self.items.push(Item::Glyphs {
            rect,
            baseline,
            node: text.node,
            font,
            size: text.size,
            color: text.color,
            glyphs: start..self.glyphs.len() as u32,
        });
        let metrics = self.source.metrics(text.font);
        let scale = text.size / metrics.units_per_em;
        let decoration =
            |offset: f32, thickness: f32| (offset * scale, (thickness * scale).max(1.0));
        let decorations = [
            (
                text.decorations.underline,
                decoration(metrics.underline_offset, metrics.underline_thickness),
            ),
            (
                text.decorations.line_through,
                decoration(metrics.strikeout_offset, metrics.strikeout_thickness),
            ),
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

fn snap(rect: Rect) -> Rect {
    let (left, top) = (rect.x.round(), rect.y.round());
    let (right, bottom) = (
        (rect.x + rect.width).round(),
        (rect.y + rect.height).round(),
    );
    Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}
