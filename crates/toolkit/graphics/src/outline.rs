use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use skrifa::bitmap::{BitmapData, Origin};
use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, GlyphId, MetadataProvider, Tag};
use vello_cpu::kurbo::{self, Affine, BezPath, Vec2};

use crate::scene::{Image, Rect};
use crate::text::GlyphRun;

pub(crate) const TOLERANCE: f64 = 0.004;

const COLOR_TABLES: [Tag; 4] = [
    Tag::new(b"COLR"),
    Tag::new(b"CBDT"),
    Tag::new(b"sbix"),
    Tag::new(b"SVG "),
];
const UNUSED_FRAMES: u64 = 64;
const KNOWN_BOUNDS: usize = 16384;
const COLOUR_GLYPH: kurbo::Rect = kurbo::Rect::new(-1.0, -2.0, 2.0, 1.0);
const LARGEST_BITMAP: u32 = 2048;

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
    bitmaps: HashMap<GlyphKey, Entry<GlyphBitmap>>,
    scale: f64,
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
            .or_insert_with(|| has_color_tables(run))
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

    pub(crate) fn bitmap(&mut self, run: &GlyphRun, glyph: u32) -> Option<Arc<GlyphBitmap>> {
        let (frame, scale) = (self.frame, self.scale);
        let entry = self
            .bitmaps
            .entry(glyph_key(run, glyph))
            .or_insert_with(|| Entry {
                drawn: bitmap(run, glyph, scale).map(Arc::new),
                used: frame,
            });
        entry.used = frame;
        entry.drawn.clone()
    }

    pub(crate) fn rescale(&mut self, scale: f64) -> bool {
        let rescaled = self.scale != scale;
        if rescaled {
            self.scale = scale;
            self.bitmaps.clear();
        }
        rescaled
    }

    pub(crate) fn end_frame(&mut self) {
        let frame = self.frame;
        self.glyphs
            .retain(|_, entry| frame - entry.used < UNUSED_FRAMES);
        self.bitmaps
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

fn has_color_tables(run: &GlyphRun) -> bool {
    FontRef::from_index(run.font_bytes(), run.font_index()).is_ok_and(|font| {
        COLOR_TABLES
            .iter()
            .any(|tag| font.table_data(*tag).is_some())
    })
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

fn bitmap(run: &GlyphRun, glyph: u32, scale: f64) -> Option<GlyphBitmap> {
    let font = FontRef::from_index(run.font_bytes(), run.font_index()).ok()?;
    let id = GlyphId::new(glyph);
    if font.color_glyphs().get(id).is_some() {
        return None;
    }
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
    let mut pixels = reader.decode().ok()?.into_rgba8();
    for pixel in pixels.pixels_mut() {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
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
    let (mut path, units_per_em) = unscaled(run, glyph)?;
    path.apply_affine(run_transform(run) * Affine::scale(1.0 / units_per_em));
    let mut lines = BezPath::new();
    kurbo::flatten(path, TOLERANCE, |element| lines.push(element));
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

/// The area the glyphs of `runs` paint.
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
                .or_insert_with(|| (has_color_tables(run), HashMap::new()));
            let known_before = known.len();
            let transform = run_transform(run);
            for glyph in &run.glyphs {
                let within_em = match *coloured {
                    true => Some(COLOUR_GLYPH),
                    false => em_bounds(known, run, glyph.id),
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
