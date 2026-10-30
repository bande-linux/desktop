use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use parley::fontique::GenericFamily;
use parley::{
    Alignment, AlignmentOptions, FontContext, FontData, FontFamily, FontFamilyName, FontWeight,
    Layout, LayoutContext, LineHeight, PositionedLayoutItem, RangedBuilder, StyleProperty,
};
use unbound_design::{Color, TextRole, Typeface};

const DISPLAY_SIZE: f32 = 20.0;

static INTERFACE_FAMILIES: [FontFamilyName<'static>; 2] = [
    FontFamilyName::named(Typeface::Interface.family()),
    FontFamilyName::Generic(GenericFamily::SansSerif),
];

static DISPLAY_FAMILIES: [FontFamilyName<'static>; 3] = [
    FontFamilyName::named(Typeface::Display.family()),
    FontFamilyName::named(Typeface::Interface.family()),
    FontFamilyName::Generic(GenericFamily::SansSerif),
];

static MONOSPACE_FAMILIES: [FontFamilyName<'static>; 3] = [
    FontFamilyName::named(Typeface::Monospace.family()),
    FontFamilyName::named("DejaVu Sans Mono"),
    FontFamilyName::Generic(GenericFamily::Monospace),
];

fn families(typeface: Typeface) -> FontFamily<'static> {
    let families = match typeface {
        Typeface::Interface => INTERFACE_FAMILIES.as_slice(),
        Typeface::Display => DISPLAY_FAMILIES.as_slice(),
        Typeface::Monospace => MONOSPACE_FAMILIES.as_slice(),
    };
    FontFamily::List(families.into())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub weight: f32,
    pub color: Color,
    pub typeface: Typeface,
    pub tabular: bool,
    pub line_height: Option<f32>,
    pub tracking: f32,
    pub caps: bool,
}

impl TextStyle {
    pub const REGULAR: f32 = 400.0;
    pub const MEDIUM: f32 = 500.0;
    pub const SEMIBOLD: f32 = 600.0;
    pub const BOLD: f32 = 700.0;

    pub const fn new(size: f32, color: Color) -> Self {
        Self {
            size,
            weight: Self::REGULAR,
            color,
            typeface: if size >= DISPLAY_SIZE {
                Typeface::Display
            } else {
                Typeface::Interface
            },
            tabular: false,
            line_height: None,
            tracking: 0.0,
            caps: false,
        }
    }

    pub const fn role(role: TextRole, color: Color) -> Self {
        let spec = role.spec();
        Self {
            size: spec.size,
            weight: spec.weight,
            color,
            typeface: spec.typeface,
            tabular: false,
            line_height: Some(spec.line_height),
            tracking: spec.tracking,
            caps: spec.caps,
        }
    }

    pub const fn tabular(self) -> Self {
        Self {
            tabular: true,
            ..self
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

    pub fn apply(&self, builder: &mut RangedBuilder<'_, Color>) {
        builder.push_default(StyleProperty::FontFamily(families(self.typeface)));
        builder.push_default(StyleProperty::FontSize(self.size));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(self.weight)));
        builder.push_default(StyleProperty::Brush(self.color));
        builder.push_default(StyleProperty::FontFeatures(
            self.typeface.features(self.tabular).into(),
        ));
        if let Some(line_height) = self.line_height {
            builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(line_height)));
        }
        if self.tracking != 0.0 {
            builder.push_default(StyleProperty::LetterSpacing(self.tracking * self.size));
        }
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

    pub fn sharing_fonts(&self) -> Self {
        Self {
            fonts: self.fonts.clone(),
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
        let text = if style.caps {
            Cow::Owned(text.to_uppercase())
        } else {
            Cow::Borrowed(text)
        };
        let mut builder = self
            .layouts
            .ranged_builder(&mut self.fonts, &text, 1.0, true);
        style.apply(&mut builder);
        let mut layout = builder.build(&text);
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

impl PartialEq for GlyphRun {
    fn eq(&self, other: &Self) -> bool {
        self.font_blob_id() == other.font_blob_id()
            && self.font.index == other.font.index
            && self.size == other.size
            && self.color == other.color
            && self.skew_degrees == other.skew_degrees
            && self.coords == other.coords
            && self.glyphs == other.glyphs
    }
}

impl PartialEq for Paragraph {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.layout, &other.layout) || self.glyph_runs() == other.glyph_runs()
    }
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

    pub fn layout(&self) -> &Layout<Color> {
        &self.layout
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
