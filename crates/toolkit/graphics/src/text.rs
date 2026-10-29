use std::fmt;
use std::sync::Arc;

use parley::fontique::GenericFamily;
use parley::{
    Alignment, AlignmentOptions, FontContext, FontData, FontFamily, FontFamilyName, FontWeight,
    Layout, LayoutContext, PositionedLayoutItem, StyleProperty,
};

use crate::tokens::Color;

pub const FONT_FAMILY: &str = "Inter";
pub const DISPLAY_FONT_FAMILY: &str = "Inter Display";
pub const DISPLAY_SIZE: f32 = 20.0;
pub const MONOSPACE_FONT_FAMILY: &str = "DejaVu Sans Mono";

static MONOSPACE_FAMILIES: [FontFamilyName<'static>; 2] = [
    FontFamilyName::named(MONOSPACE_FONT_FAMILY),
    FontFamilyName::Generic(GenericFamily::Monospace),
];

static TEXT_FAMILIES: [FontFamilyName<'static>; 2] = [
    FontFamilyName::named(FONT_FAMILY),
    FontFamilyName::Generic(GenericFamily::SansSerif),
];

static DISPLAY_FAMILIES: [FontFamilyName<'static>; 3] = [
    FontFamilyName::named(DISPLAY_FONT_FAMILY),
    FontFamilyName::named(FONT_FAMILY),
    FontFamilyName::Generic(GenericFamily::SansSerif),
];

pub fn families(size: f32) -> FontFamily<'static> {
    let families = if size >= DISPLAY_SIZE {
        DISPLAY_FAMILIES.as_slice()
    } else {
        TEXT_FAMILIES.as_slice()
    };
    FontFamily::List(families.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Typeface {
    #[default]
    Interface,
    Monospace,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub weight: f32,
    pub color: Color,
    pub typeface: Typeface,
}

impl TextStyle {
    pub const REGULAR: f32 = 400.0;
    pub const MEDIUM: f32 = 500.0;
    pub const SEMIBOLD: f32 = 600.0;

    pub const fn new(size: f32, color: Color) -> Self {
        Self {
            size,
            weight: Self::REGULAR,
            color,
            typeface: Typeface::Interface,
        }
    }

    pub const fn monospace(self) -> Self {
        Self {
            typeface: Typeface::Monospace,
            ..self
        }
    }

    pub const fn weight(self, weight: f32) -> Self {
        Self { weight, ..self }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
}

pub struct TextSystem {
    fonts: FontContext,
    layouts: LayoutContext<Color>,
}

impl Default for TextSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl TextSystem {
    pub fn new() -> Self {
        Self {
            fonts: FontContext::new(),
            layouts: LayoutContext::new(),
        }
    }

    pub fn contexts(&mut self) -> (&mut FontContext, &mut LayoutContext<Color>) {
        (&mut self.fonts, &mut self.layouts)
    }

    pub fn paragraph(
        &mut self,
        text: &str,
        style: TextStyle,
        max_width: Option<f32>,
        align: TextAlign,
    ) -> Paragraph {
        let mut builder = self
            .layouts
            .ranged_builder(&mut self.fonts, text, 1.0, true);
        let families = match style.typeface {
            Typeface::Monospace => MONOSPACE_FAMILIES.as_slice(),
            Typeface::Interface if style.size >= DISPLAY_SIZE => DISPLAY_FAMILIES.as_slice(),
            Typeface::Interface => TEXT_FAMILIES.as_slice(),
        };
        builder.push_default(StyleProperty::FontFamily(FontFamily::List(families.into())));
        builder.push_default(StyleProperty::FontSize(style.size));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(style.weight)));
        builder.push_default(StyleProperty::Brush(style.color));
        let mut layout = builder.build(text);
        layout.break_all_lines(max_width);
        let alignment = match align {
            TextAlign::Start => Alignment::Start,
            TextAlign::Center => Alignment::Center,
            TextAlign::End => Alignment::End,
        };
        layout.align(alignment, AlignmentOptions::default());
        Paragraph {
            layout: Arc::new(layout),
        }
    }
}

#[derive(Clone)]
pub struct Paragraph {
    layout: Arc<Layout<Color>>,
}

impl fmt::Debug for Paragraph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Paragraph")
            .field("width", &self.width())
            .field("height", &self.height())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedGlyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone)]
pub struct GlyphRun {
    pub(crate) font: FontData,
    pub size: f32,
    pub color: Color,
    pub skew_degrees: Option<f32>,
    pub(crate) coords: Vec<i16>,
    pub glyphs: Vec<PositionedGlyph>,
}

impl GlyphRun {
    pub fn font_bytes(&self) -> &[u8] {
        self.font.data.data()
    }

    pub fn font_index(&self) -> u32 {
        self.font.index
    }

    pub(crate) fn font_blob_id(&self) -> u64 {
        self.font.data.id()
    }

    pub fn variation_coords(&self) -> &[i16] {
        &self.coords
    }
}

impl Paragraph {
    pub fn from_layout(layout: Layout<Color>) -> Self {
        Self {
            layout: Arc::new(layout),
        }
    }

    pub fn width(&self) -> f32 {
        self.layout.width()
    }

    pub fn height(&self) -> f32 {
        self.layout.height()
    }

    pub fn line_count(&self) -> usize {
        self.layout.len()
    }

    pub fn glyph_runs(&self) -> Vec<GlyphRun> {
        let mut runs = Vec::new();
        for line in self.layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                let run = glyph_run.run();
                runs.push(GlyphRun {
                    font: run.font().clone(),
                    size: run.font_size(),
                    color: glyph_run.style().brush,
                    skew_degrees: run.synthesis().skew(),
                    coords: run
                        .normalized_coords()
                        .iter()
                        .map(|coord| coord.to_bits())
                        .collect(),
                    glyphs: glyph_run
                        .positioned_glyphs()
                        .map(|glyph| PositionedGlyph {
                            id: glyph.id,
                            x: glyph.x,
                            y: glyph.y,
                        })
                        .collect(),
                });
            }
        }
        runs
    }
}
