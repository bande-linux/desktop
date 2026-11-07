use std::collections::HashMap;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use bande_graphics::{
    Blend, Color, GlyphRun, Image, Item, Paint, Paragraph, Path, PathElement, Point, Rect, Shape,
    Stop, TextRun, Transform,
};
use krilla::Document;
use krilla::blend::BlendMode;
use krilla::color::rgb;
use krilla::geom::{self, PathBuilder, Size};
use krilla::image::Image as PdfImage;
use krilla::metadata::Metadata;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{
    Fill, FillRule, LinearGradient, Paint as PdfPaint, RadialGradient, SpreadMethod, Stroke,
    SweepGradient,
};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, Tag};

use crate::page::{Page, PageItem};

const ARC_HANDLE: f32 = 0.552_284_8;
const RADIANS_PER_DEGREE: f32 = std::f32::consts::PI / 180.0;

#[derive(Debug)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot make the PDF: {}", self.0)
    }
}

impl std::error::Error for Error {}

pub fn write(title: &str, pages: &[Page]) -> Result<Vec<u8>, Error> {
    let mut document = Document::new();
    document.set_metadata(Metadata::new().title(title.to_owned()));
    let mut fonts = Fonts::default();
    for page in pages {
        let (width, height) = page.size;
        let size = Size::from_wh(width, height)
            .ok_or_else(|| Error(format!("a page cannot be {width} by {height}")))?;
        let mut pdf_page = document.start_page_with(PageSettings::new(size));
        let mut surface = pdf_page.surface();
        for item in &page.items {
            match item {
                PageItem::Scene(item) => draw(&mut surface, &mut fonts, item),
                PageItem::Lines {
                    origin,
                    paragraph,
                    lines,
                } => draw_lines(
                    &mut surface,
                    &mut fonts,
                    *origin,
                    paragraph,
                    Some(lines.clone()),
                ),
            }
        }
        surface.finish();
        pdf_page.finish();
    }
    document
        .finish()
        .map_err(|error| Error(format!("{error:?}")))
}

type FontKey = (u64, u32, Vec<(u32, u32)>);

#[derive(Default)]
struct Fonts(HashMap<FontKey, Option<Font>>);

impl Fonts {
    fn of(&mut self, run: &GlyphRun, variations: &[([u8; 4], f32)]) -> Option<Font> {
        let key = (
            run.font_id(),
            run.font_index(),
            variations
                .iter()
                .map(|(tag, value)| (u32::from_be_bytes(*tag), value.to_bits()))
                .collect(),
        );
        self.0
            .entry(key)
            .or_insert_with(|| {
                let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(run.font_blob());
                let settings = variations
                    .iter()
                    .map(|(tag, value)| (Tag::new(tag), *value))
                    .collect::<Vec<_>>();
                Font::new_variable(data.into(), run.font_index(), &settings)
            })
            .clone()
    }
}

struct PlacedGlyph {
    id: u32,
    text: Range<usize>,
    x_advance: f32,
    y_advance: f32,
}

impl krilla::text::Glyph for PlacedGlyph {
    fn glyph_id(&self) -> GlyphId {
        GlyphId::new(self.id)
    }

    fn text_range(&self) -> Range<usize> {
        self.text.clone()
    }

    fn x_advance(&self, size: f32) -> f32 {
        self.x_advance * size
    }

    fn x_offset(&self, _size: f32) -> f32 {
        0.0
    }

    fn y_offset(&self, _size: f32) -> f32 {
        0.0
    }

    fn y_advance(&self, size: f32) -> f32 {
        self.y_advance * size
    }

    fn location(&self) -> Option<krilla::surface::Location> {
        None
    }
}

fn opacity(value: f32) -> NormalizedF32 {
    NormalizedF32::new(value.clamp(0.0, 1.0)).unwrap_or(NormalizedF32::ONE)
}

fn rgb_of(color: Color) -> rgb::Color {
    rgb::Color::new(color.r, color.g, color.b)
}

fn alpha_of(color: Color) -> f32 {
    f32::from(color.a) / 255.0
}

fn stops(stops: &[Stop]) -> Vec<krilla::paint::Stop> {
    stops
        .iter()
        .map(|stop| krilla::paint::Stop {
            offset: opacity(stop.offset),
            color: rgb_of(stop.color).into(),
            opacity: opacity(alpha_of(stop.color)),
        })
        .collect()
}

fn paint_of(paint: &Paint) -> (PdfPaint, f32) {
    match paint {
        Paint::Solid(color) => (rgb_of(*color).into(), alpha_of(*color)),
        Paint::LinearGradient {
            start,
            end,
            stops: colours,
        } => (
            LinearGradient {
                x1: start.x,
                y1: start.y,
                x2: end.x,
                y2: end.y,
                transform: geom::Transform::identity(),
                spread_method: SpreadMethod::Pad,
                stops: stops(colours),
                anti_alias: true,
            }
            .into(),
            1.0,
        ),
        Paint::RadialGradient {
            center,
            radius,
            stops: colours,
        } => (
            RadialGradient {
                fx: center.x,
                fy: center.y,
                fr: 0.0,
                cx: center.x,
                cy: center.y,
                cr: *radius,
                transform: geom::Transform::identity(),
                spread_method: SpreadMethod::Pad,
                stops: stops(colours),
                anti_alias: true,
            }
            .into(),
            1.0,
        ),
        Paint::ConicGradient {
            center,
            start_angle,
            end_angle,
            stops: colours,
        } => (
            SweepGradient {
                cx: center.x,
                cy: center.y,
                start_angle: start_angle.to_degrees(),
                end_angle: end_angle.to_degrees(),
                transform: geom::Transform::identity(),
                spread_method: SpreadMethod::Pad,
                stops: stops(colours),
                anti_alias: true,
            }
            .into(),
            1.0,
        ),
    }
}

fn fill_with(paint: &Paint) -> Fill {
    let (paint, alpha) = paint_of(paint);
    Fill {
        paint,
        opacity: opacity(alpha),
        rule: FillRule::NonZero,
    }
}

fn stroke_with(paint: &Paint, width: f32) -> Stroke {
    let (paint, alpha) = paint_of(paint);
    Stroke {
        paint,
        width,
        opacity: opacity(alpha),
        ..Stroke::default()
    }
}

fn shape_path(shape: Shape) -> Option<geom::Path> {
    let Rect {
        x,
        y,
        width,
        height,
    } = shape.rect;
    let radius = shape.radius.min(width / 2.0).min(height / 2.0).max(0.0);
    let (right, bottom) = (x + width, y + height);
    let handle = radius * (1.0 - ARC_HANDLE);
    let mut path = PathBuilder::new();
    path.move_to(x + radius, y);
    path.line_to(right - radius, y);
    path.cubic_to(right - handle, y, right, y + handle, right, y + radius);
    path.line_to(right, bottom - radius);
    path.cubic_to(
        right,
        bottom - handle,
        right - handle,
        bottom,
        right - radius,
        bottom,
    );
    path.line_to(x + radius, bottom);
    path.cubic_to(x + handle, bottom, x, bottom - handle, x, bottom - radius);
    path.line_to(x, y + radius);
    path.cubic_to(x, y + handle, x + handle, y, x + radius, y);
    path.close();
    path.finish()
}

fn free_path(path: &Path) -> Option<geom::Path> {
    let mut built = PathBuilder::new();
    for element in path.elements() {
        match element {
            PathElement::Move(point) => built.move_to(point.x, point.y),
            PathElement::Line(point) => built.line_to(point.x, point.y),
            PathElement::Cubic(first, second, end) => {
                built.cubic_to(first.x, first.y, second.x, second.y, end.x, end.y);
            }
            PathElement::Close => built.close(),
        }
    }
    built.finish()
}

fn blend_of(mode: Blend) -> BlendMode {
    use Blend as Mix;
    match mode {
        Mix::Multiply => BlendMode::Multiply,
        Mix::Screen => BlendMode::Screen,
        Mix::Overlay => BlendMode::Overlay,
        Mix::Darken => BlendMode::Darken,
        Mix::Lighten => BlendMode::Lighten,
        Mix::ColorDodge => BlendMode::ColorDodge,
        Mix::ColorBurn => BlendMode::ColorBurn,
        Mix::HardLight => BlendMode::HardLight,
        Mix::SoftLight => BlendMode::SoftLight,
        Mix::Difference => BlendMode::Difference,
        Mix::Exclusion => BlendMode::Exclusion,
        Mix::Hue => BlendMode::Hue,
        Mix::Saturation => BlendMode::Saturation,
        Mix::Color => BlendMode::Color,
        Mix::Luminosity => BlendMode::Luminosity,
        _ => BlendMode::Normal,
    }
}

fn fill_path(surface: &mut Surface<'_>, path: Option<geom::Path>, paint: &Paint) {
    if let Some(path) = path {
        surface.set_stroke(None);
        surface.set_fill(Some(fill_with(paint)));
        surface.draw_path(&path);
    }
}

fn stroke_path(surface: &mut Surface<'_>, path: Option<geom::Path>, width: f32, paint: &Paint) {
    if let Some(path) = path {
        surface.set_fill(None);
        surface.set_stroke(Some(stroke_with(paint, width)));
        surface.draw_path(&path);
    }
}

fn transformed(
    surface: &mut Surface<'_>,
    transform: Transform,
    body: impl FnOnce(&mut Surface<'_>),
) {
    let [sx, ky, kx, sy, tx, ty] = transform.matrix;
    surface.push_transform(&geom::Transform::from_row(sx, ky, kx, sy, tx, ty));
    body(surface);
    surface.pop();
}

fn draw_image(surface: &mut Surface<'_>, rect: Rect, image: &Image) {
    let Some(size) = Size::from_wh(rect.width, rect.height) else {
        return;
    };
    let mut rgba = image.premultiplied_rgba();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = (u16::from(*channel) * 255 / alpha).min(255) as u8;
            }
        }
    }
    let picture = PdfImage::from_rgba8(rgba, image.width(), image.height());
    transformed(surface, Transform::translate(rect.x, rect.y), |surface| {
        surface.draw_image(picture, size);
    });
}

fn draw_tiles(surface: &mut Surface<'_>, rect: Rect, tile: Rect, image: &Image) {
    if tile.width <= 0.0 || tile.height <= 0.0 {
        return;
    }
    let Some(area) = shape_path(Shape::rect(rect)) else {
        return;
    };
    surface.push_clip_path(&area, &FillRule::NonZero);
    let mut y = rect.y + tile.y;
    while y < rect.y + rect.height {
        let mut x = rect.x + tile.x;
        while x < rect.x + rect.width {
            draw_image(surface, Rect::new(x, y, tile.width, tile.height), image);
            x += tile.width;
        }
        y += tile.height;
    }
    surface.pop();
}

fn draw_glyph_run(
    surface: &mut Surface<'_>,
    fonts: &mut Fonts,
    origin: Point,
    run: &GlyphRun,
    clusters: &[Range<usize>],
    text: &str,
    variations: &[([u8; 4], f32)],
) {
    let Some(font) = fonts.of(run, variations) else {
        return;
    };
    let Some(first) = run.glyphs.first() else {
        return;
    };
    let glyphs = run
        .glyphs
        .iter()
        .enumerate()
        .map(|(index, glyph)| {
            let next = run.glyphs.get(index + 1);
            PlacedGlyph {
                id: glyph.id,
                text: clusters.get(index).cloned().unwrap_or(0..0),
                x_advance: next.map_or(0.0, |next| (next.x - glyph.x) / run.size),
                y_advance: next.map_or(0.0, |next| (next.y - glyph.y) / run.size),
            }
        })
        .collect::<Vec<_>>();
    let start = geom::Point::from_xy(origin.x + first.x, origin.y + first.y);
    surface.set_stroke(None);
    surface.set_fill(Some(fill_with(&Paint::Solid(run.color))));
    match run.skew_degrees {
        Some(degrees) => {
            let skew = -(degrees * RADIANS_PER_DEGREE).tan();
            let shear = Transform {
                matrix: [1.0, 0.0, skew, 1.0, -skew * start.y, 0.0],
            };
            transformed(surface, shear, |surface| {
                surface.draw_glyphs(start, &glyphs, font, text, run.size, false);
            });
        }
        None => {
            surface.draw_glyphs(start, &glyphs, font, text, run.size, false);
        }
    }
}

fn draw_text_run(
    surface: &mut Surface<'_>,
    fonts: &mut Fonts,
    origin: Point,
    run: &TextRun,
    text: &str,
) {
    draw_glyph_run(
        surface,
        fonts,
        origin,
        &run.glyphs,
        &run.clusters,
        text,
        &run.variations,
    );
    for (rect, color) in &run.lines {
        let shape = Shape::rect(Rect::new(
            rect.x + origin.x,
            rect.y + origin.y,
            rect.width,
            rect.height,
        ));
        fill_path(surface, shape_path(shape), &Paint::Solid(*color));
    }
}

fn draw_lines(
    surface: &mut Surface<'_>,
    fonts: &mut Fonts,
    origin: Point,
    paragraph: &Paragraph,
    lines: Option<Range<usize>>,
) {
    let lines = lines.unwrap_or(0..paragraph.line_count());
    for run in paragraph.text_runs(lines) {
        draw_text_run(surface, fonts, origin, &run, paragraph.text());
    }
}

fn draw_all(surface: &mut Surface<'_>, fonts: &mut Fonts, items: &[Item]) {
    for item in items {
        draw(surface, fonts, item);
    }
}

fn draw(surface: &mut Surface<'_>, fonts: &mut Fonts, item: &Item) {
    match item {
        Item::Fill { shape, paint } => fill_path(surface, shape_path(*shape), paint),
        Item::Stroke {
            shape,
            width,
            paint,
        } => stroke_path(surface, shape_path(*shape), *width, paint),
        Item::Text { origin, paragraph } => draw_lines(surface, fonts, *origin, paragraph, None),
        Item::Glyphs(run) => {
            draw_glyph_run(surface, fonts, Point::default(), run, &[], "", &[]);
        }
        Item::Clip { shape, items } => {
            if let Some(area) = shape_path(*shape) {
                surface.push_clip_path(&area, &FillRule::NonZero);
                draw_all(surface, fonts, items);
                surface.pop();
            }
        }
        Item::Translate { offset, items } => {
            transformed(
                surface,
                Transform::translate(offset.x, offset.y),
                |surface| {
                    draw_all(surface, fonts, items);
                },
            );
        }
        Item::Opacity { alpha, items } => {
            surface.push_opacity(opacity(*alpha));
            draw_all(surface, fonts, items);
            surface.pop();
        }
        Item::Blend { mode, items } => {
            surface.push_blend_mode(blend_of(*mode));
            draw_all(surface, fonts, items);
            surface.pop();
        }
        Item::Transform { transform, items } => {
            transformed(surface, *transform, |surface| {
                draw_all(surface, fonts, items);
            });
        }
        Item::Line {
            from,
            to,
            width,
            paint,
        } => stroke_path(
            surface,
            free_path(&Path::through(&[*from, *to])),
            *width,
            paint,
        ),
        Item::FillPath { path, paint } => fill_path(surface, free_path(path), paint),
        Item::StrokePath { path, width, paint } => {
            stroke_path(surface, free_path(path), *width, paint);
        }
        Item::Scale {
            origin,
            factor,
            items,
        } => {
            let scale = Transform::translate(-origin.x, -origin.y)
                .then(Transform::scale(*factor, *factor))
                .then(Transform::translate(origin.x, origin.y));
            transformed(surface, scale, |surface| {
                draw_all(surface, fonts, items);
            });
        }
        Item::Image { rect, image } => draw_image(surface, *rect, image),
        Item::Tiles { rect, tile, image } => draw_tiles(surface, *rect, *tile, image),
        Item::Picture(picture) => draw_all(surface, fonts, picture.items()),
        Item::Shadow { .. } | Item::CastShadow { .. } | Item::Surface { .. } => {}
    }
}

#[cfg(feature = "pages")]
pub fn keep_pages(pdf: &[u8], pages: &[usize]) -> Result<Vec<u8>, Error> {
    use krilla::pdf::{Pdf, PdfDocument};
    let source = Pdf::new(Arc::new(pdf.to_vec())).map_err(|error| Error(format!("{error:?}")))?;
    let mut document = Document::new();
    document.embed_pdf_pages(&PdfDocument::new(Arc::new(source)), pages);
    document
        .finish()
        .map_err(|error| Error(format!("{error:?}")))
}
