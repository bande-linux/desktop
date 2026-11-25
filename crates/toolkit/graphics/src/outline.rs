use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use bande_design::Color;
use skrifa::bitmap::{BitmapData, Origin};
use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, GlyphId, MetadataProvider, Tag};
use vello_cpu::kurbo::{self, Affine, BezPath, Vec2};

use crate::colr;
use crate::scene::{Image, Rect, premultiply};
use crate::text::GlyphRun;

pub(crate) const TOLERANCE: f64 = 0.004;

const LAYERS_TABLE: Tag = Tag::new(b"COLR");
const COLOR_TABLES: [Tag; 4] = [
    LAYERS_TABLE,
    Tag::new(b"CBDT"),
    Tag::new(b"sbix"),
    Tag::new(b"SVG "),
];
const UNUSED_FRAMES: u64 = 64;
const KNOWN_BOUNDS: usize = 16384;
const COLOUR_GLYPH: kurbo::Rect = kurbo::Rect::new(-1.0, -2.0, 2.0, 1.0);
const GLYPH_TOLERANCE: f64 = 1e-5;
const LARGEST_SIZE: f32 = 1e6;
const STEEPEST_SKEW: f32 = 89.0;
pub(crate) const LARGEST_BITMAP: u32 = 2048;

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

pub(crate) struct GlyphOutline {
    pub(crate) path: BezPath,
    pub(crate) bounds: kurbo::Rect,
}

pub(crate) struct GlyphBitmap {
    pub(crate) image: Image,
    area: Rect,
    on_pixels: bool,
}

struct Entry<T> {
    drawn: Option<Arc<T>>,
    used: u64,
}

#[derive(Default)]
pub(crate) struct GlyphOutlines {
    glyphs: HashMap<GlyphKey, Entry<GlyphOutline>>,
    coloured: HashMap<(GlyphKey, Option<Color>), Entry<GlyphBitmap>>,
    scale: f64,
    color_fonts: HashMap<FontKey, bool>,
    layered_fonts: HashMap<FontKey, bool>,
    frame: u64,
}

#[derive(Default)]
pub(crate) struct Pen(pub(crate) BezPath);

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

impl GlyphBitmap {
    pub(crate) fn placed(&self, x: f32, y: f32, scale: f64) -> kurbo::Rect {
        let (x, y) = (
            f64::from(x) + f64::from(self.area.x),
            f64::from(y) + f64::from(self.area.y),
        );
        if !self.on_pixels {
            let size = (f64::from(self.area.width), f64::from(self.area.height));
            return kurbo::Rect::from_origin_size((x, y), size);
        }
        let on_pixel = |at: f64| (at * scale).round() / scale;
        let length = |pixels: u32| f64::from(pixels) / scale;
        kurbo::Rect::from_origin_size(
            (on_pixel(x), on_pixel(y)),
            (length(self.image.width()), length(self.image.height())),
        )
    }
}

impl GlyphOutlines {
    pub(crate) fn scale(&self) -> f64 {
        self.scale
    }

    pub(crate) fn is_color_font(&mut self, run: &GlyphRun) -> bool {
        *self
            .color_fonts
            .entry(font_key(run))
            .or_insert_with(|| has_any_table(run, &COLOR_TABLES))
    }

    fn is_layered_font(&mut self, run: &GlyphRun) -> bool {
        *self
            .layered_fonts
            .entry(font_key(run))
            .or_insert_with(|| has_any_table(run, &[LAYERS_TABLE]))
    }

    pub(crate) fn outline(&mut self, run: &GlyphRun, glyph: u32) -> Option<Arc<GlyphOutline>> {
        let frame = self.frame;
        let entry = self
            .glyphs
            .entry(glyph_key(run, glyph))
            .or_insert_with(|| Entry {
                drawn: draw(run, glyph).map(|path| {
                    let bounds = path.control_box();
                    Arc::new(GlyphOutline { path, bounds })
                }),
                used: frame,
            });
        entry.used = frame;
        entry.drawn.clone()
    }

    pub(crate) fn coloured(&mut self, run: &GlyphRun, glyph: u32) -> Option<Arc<GlyphBitmap>> {
        let (frame, scale) = (self.frame, self.scale);
        let foreground = self.is_layered_font(run).then_some(run.color);
        let entry = self
            .coloured
            .entry((glyph_key(run, glyph), foreground))
            .or_insert_with(|| Entry {
                drawn: coloured(run, glyph, scale).map(Arc::new),
                used: frame,
            });
        entry.used = frame;
        entry.drawn.clone()
    }

    pub(crate) fn rescale(&mut self, scale: f64) -> bool {
        let rescaled = self.scale != scale;
        if rescaled {
            self.scale = scale;
            self.coloured.clear();
        }
        rescaled
    }

    pub(crate) fn end_frame(&mut self) {
        let frame = self.frame;
        self.glyphs
            .retain(|_, entry| frame - entry.used < UNUSED_FRAMES);
        self.coloured
            .retain(|_, entry| frame - entry.used < UNUSED_FRAMES);
        self.frame += 1;
    }
}

fn font_key(run: &GlyphRun) -> FontKey {
    FontKey {
        blob: run.font_id(),
        index: run.font_index(),
    }
}

fn has_any_table(run: &GlyphRun, tables: &[Tag]) -> bool {
    FontRef::from_index(run.font_bytes(), run.font_index())
        .is_ok_and(|font| tables.iter().any(|tag| font.table_data(*tag).is_some()))
}

fn glyph_key(run: &GlyphRun, glyph: u32) -> GlyphKey {
    GlyphKey {
        font: font_key(run),
        glyph,
        size: run.size.to_bits(),
        skew: run.skew_degrees.map(f32::to_bits),
        coords: run.variation_coords().to_vec(),
    }
}

fn drawable(run: &GlyphRun) -> bool {
    run.size > 0.0
        && run.size <= LARGEST_SIZE
        && run
            .skew_degrees
            .is_none_or(|degrees| degrees.abs() < STEEPEST_SKEW)
}

fn coloured(run: &GlyphRun, glyph: u32, scale: f64) -> Option<GlyphBitmap> {
    if !drawable(run) {
        return None;
    }
    let font = FontRef::from_index(run.font_bytes(), run.font_index()).ok()?;
    let id = GlyphId::new(glyph);
    match font.color_glyphs().get(id) {
        Some(_) => layers(&font, run, id, scale),
        None => bitmap(&font, run, id, scale),
    }
}

fn layers(font: &FontRef<'_>, run: &GlyphRun, id: GlyphId, scale: f64) -> Option<GlyphBitmap> {
    let painted = colr::paint(font, run, id, scale)?;
    let (width, height) = painted.size;
    let area = Rect::new(
        painted.area.x0 as f32,
        painted.area.y0 as f32,
        painted.area.width() as f32,
        painted.area.height() as f32,
    );
    let on_pixels = (painted.area.width() * scale - f64::from(width)).abs() < 0.5;
    Some(GlyphBitmap {
        image: Image::from_premultiplied(width, height, painted.pixels)?,
        area,
        on_pixels,
    })
}

fn bitmap(font: &FontRef<'_>, run: &GlyphRun, id: GlyphId, scale: f64) -> Option<GlyphBitmap> {
    let strike = font
        .bitmap_strikes()
        .glyph_for_size(Size::new(run.size), id)?;
    let BitmapData::Png(png) = strike.data else {
        return None;
    };
    let mut reader = image::ImageReader::with_format(Cursor::new(png), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(LARGEST_BITMAP);
    limits.max_image_height = Some(LARGEST_BITMAP);
    reader.limits(limits);
    let mut pixels = match reader.decode() {
        Ok(decoded) => decoded.into_rgba8(),
        Err(error) => {
            tracing::warn!("cannot read a glyph's bitmap: {error}");
            return None;
        }
    };
    premultiply(&mut pixels);
    let (width, height) = pixels.dimensions();
    let (across, down) = (run.size / strike.ppem_x, run.size / strike.ppem_y);
    let em = run.size / f32::from(font.head().ok()?.units_per_em());
    let top = match strike.placement_origin {
        Origin::TopLeft => 0.0,
        Origin::BottomLeft => -(height as f32),
    };
    let area = Rect::new(
        strike.bearing_x * em + strike.inner_bearing_x * across,
        -strike.bearing_y * em + (top - strike.inner_bearing_y) * down,
        width as f32 * across,
        height as f32 * down,
    );
    if ![area.x, area.y, area.width, area.height]
        .iter()
        .all(|length| length.is_finite())
    {
        return None;
    }
    let on_screen = |length: f32| ((f64::from(length) * scale).round() as u32).max(1);
    let sized = (on_screen(area.width), on_screen(area.height));
    let on_pixels = sized.0.max(sized.1) <= LARGEST_BITMAP;
    let shown = match on_pixels {
        true => sized,
        false => (width, height),
    };
    if shown != (width, height) {
        pixels = image::imageops::resize(
            &pixels,
            shown.0,
            shown.1,
            image::imageops::FilterType::Triangle,
        );
    }
    Some(GlyphBitmap {
        image: Image::from_premultiplied(shown.0, shown.1, pixels.into_raw())?,
        area,
        on_pixels,
    })
}

fn unscaled(run: &GlyphRun, glyph: u32) -> Option<(BezPath, f64)> {
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
    Some((pen.0, f64::from(font.head().ok()?.units_per_em())))
}

fn run_transform(run: &GlyphRun) -> Affine {
    let skew = match run.skew_degrees {
        Some(degrees) => Affine::skew(-f64::from(degrees.to_radians().tan()), 0.0),
        None => Affine::IDENTITY,
    };
    skew * Affine::scale(f64::from(run.size))
}

fn draw(run: &GlyphRun, glyph: u32) -> Option<BezPath> {
    if !drawable(run) {
        return None;
    }
    let (mut path, units_per_em) = unscaled(run, glyph)?;
    path.apply_affine(run_transform(run) * Affine::scale(1.0 / units_per_em));
    let tolerance = TOLERANCE.max(f64::from(run.size) * GLYPH_TOLERANCE);
    let mut lines = BezPath::new();
    kurbo::flatten(path, tolerance, |element| lines.push(element));
    Some(lines)
}

type GlyphBounds = HashMap<u32, Option<kurbo::Rect>>;

#[derive(Default)]
struct KnownBounds {
    fonts: HashMap<(FontKey, Vec<i16>), (bool, GlyphBounds)>,
    glyphs: usize,
}

thread_local! {
    static BOUNDS: RefCell<KnownBounds> = RefCell::default();
}

fn em_bounds(known: &mut GlyphBounds, run: &GlyphRun, glyph: u32) -> Option<kurbo::Rect> {
    *known.entry(glyph).or_insert_with(|| {
        let (path, units_per_em) = unscaled(run, glyph)?;
        (!path.is_empty())
            .then(|| Affine::scale(1.0 / units_per_em).transform_rect_bbox(path.control_box()))
    })
}

pub(crate) fn ink(runs: &[GlyphRun]) -> Option<Rect> {
    let painted = BOUNDS.with_borrow_mut(|bounds| {
        let mut painted: Option<kurbo::Rect> = None;
        for run in runs {
            if bounds.glyphs > KNOWN_BOUNDS {
                *bounds = KnownBounds::default();
            }
            let font = (font_key(run), run.variation_coords().to_vec());
            let (coloured, known) = bounds
                .fonts
                .entry(font)
                .or_insert_with(|| (has_any_table(run, &COLOR_TABLES), HashMap::new()));
            let known_before = known.len();
            let transform = run_transform(run);
            for glyph in &run.glyphs {
                let outline = em_bounds(known, run, glyph.id);
                let within_em = match *coloured {
                    true => Some(outline.map_or(COLOUR_GLYPH, |area| area.union(COLOUR_GLYPH))),
                    false => outline,
                };
                let Some(within_em) = within_em else {
                    continue;
                };
                let at = Vec2::new(f64::from(glyph.x), f64::from(glyph.y));
                let area = transform.transform_rect_bbox(within_em) + at;
                painted = Some(painted.map_or(area, |painted| painted.union(area)));
            }
            bounds.glyphs += known.len() - known_before;
        }
        painted
    })?;
    Some(Rect::new(
        painted.x0 as f32,
        painted.y0 as f32,
        painted.width() as f32,
        painted.height() as f32,
    ))
}
