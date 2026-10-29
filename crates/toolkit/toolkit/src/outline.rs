use std::collections::HashMap;
use std::sync::Arc;

use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, GlyphId, MetadataProvider, Tag};
use vello_cpu::kurbo::{self, Affine, BezPath};

use crate::text::GlyphRun;

pub(crate) const TOLERANCE: f64 = 0.004;

const COLOR_TABLES: [Tag; 4] = [
    Tag::new(b"COLR"),
    Tag::new(b"CBDT"),
    Tag::new(b"sbix"),
    Tag::new(b"SVG "),
];
const UNUSED_FRAMES: u64 = 64;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FontKey {
    blob: u64,
    index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GlyphKey {
    font: FontKey,
    glyph: u32,
    size: u32,
    skew: Option<u32>,
    coords: Vec<i16>,
}

struct Entry {
    path: Option<Arc<BezPath>>,
    used: u64,
}

#[derive(Default)]
pub(crate) struct GlyphOutlines {
    glyphs: HashMap<GlyphKey, Entry>,
    color_fonts: HashMap<FontKey, bool>,
    frame: u64,
}

struct Pen(BezPath);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), -f64::from(y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), -f64::from(y)));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to(
            (f64::from(cx0), -f64::from(cy0)),
            (f64::from(x), -f64::from(y)),
        );
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to(
            (f64::from(cx0), -f64::from(cy0)),
            (f64::from(cx1), -f64::from(cy1)),
            (f64::from(x), -f64::from(y)),
        );
    }

    fn close(&mut self) {
        self.0.close_path();
    }
}

impl GlyphOutlines {
    pub(crate) fn is_color_font(&mut self, run: &GlyphRun) -> bool {
        *self.color_fonts.entry(font_key(run)).or_insert_with(|| {
            FontRef::from_index(run.font_bytes(), run.font_index()).is_ok_and(|font| {
                COLOR_TABLES
                    .iter()
                    .any(|tag| font.table_data(*tag).is_some())
            })
        })
    }

    pub(crate) fn outline(&mut self, run: &GlyphRun, glyph: u32) -> Option<Arc<BezPath>> {
        let key = GlyphKey {
            font: font_key(run),
            glyph,
            size: run.size.to_bits(),
            skew: run.skew_degrees.map(f32::to_bits),
            coords: run.variation_coords().to_vec(),
        };
        let frame = self.frame;
        let entry = self.glyphs.entry(key).or_insert_with(|| Entry {
            path: draw(run, glyph).map(Arc::new),
            used: frame,
        });
        entry.used = frame;
        entry.path.clone()
    }

    pub(crate) fn end_frame(&mut self) {
        let frame = self.frame;
        self.glyphs
            .retain(|_, entry| frame - entry.used < UNUSED_FRAMES);
        self.frame += 1;
    }
}

fn font_key(run: &GlyphRun) -> FontKey {
    FontKey {
        blob: run.font_blob_id(),
        index: run.font_index(),
    }
}

fn draw(run: &GlyphRun, glyph: u32) -> Option<BezPath> {
    let font = FontRef::from_index(run.font_bytes(), run.font_index()).ok()?;
    let outline = font.outline_glyphs().get(GlyphId::new(glyph))?;
    let coords: Vec<NormalizedCoord> = run
        .variation_coords()
        .iter()
        .map(|bits| NormalizedCoord::from_bits(*bits))
        .collect();
    let mut pen = Pen(BezPath::new());
    outline
        .draw(
            DrawSettings::unhinted(Size::unscaled(), LocationRef::new(&coords)),
            &mut pen,
        )
        .ok()?;
    let units_per_em = f64::from(font.head().ok()?.units_per_em());
    let mut path = pen.0;
    path.apply_affine(Affine::scale(f64::from(run.size) / units_per_em));
    if let Some(degrees) = run.skew_degrees {
        path.apply_affine(Affine::skew(-f64::from(degrees.to_radians().tan()), 0.0));
    }
    let mut lines = BezPath::new();
    kurbo::flatten(path, TOLERANCE, |element| lines.push(element));
    Some(lines)
}
