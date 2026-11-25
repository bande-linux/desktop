use skrifa::color::{Brush, ColorPainter, ColorStop, CompositeMode, Transform};
use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::outline::DrawSettings;
use skrifa::raw::TableProvider;
use skrifa::raw::types::BoundingBox;
use skrifa::{FontRef, GlyphId, MetadataProvider};
use vello_cpu::kurbo::{self, Affine};
use vello_cpu::{Glyph, Pixmap, RenderContext, Resources};

use crate::draw;
use crate::outline::{LARGEST_BITMAP, Pen};
use crate::text::GlyphRun;

const FOREGROUND: u16 = u16::MAX;

/// Walks a colour glyph's layers to find what it paints and whether all of it can be drawn.
struct Survey<'a> {
    font: &'a FontRef<'a>,
    coords: &'a [NormalizedCoord],
    colours: usize,
    transforms: Vec<Affine>,
    clips: Vec<kurbo::Rect>,
    painted: Option<kurbo::Rect>,
    drawable: bool,
}

fn upright(area: kurbo::Rect) -> kurbo::Rect {
    kurbo::Rect::new(area.x0, -area.y1, area.x1, -area.y0)
}

impl Survey<'_> {
    fn transform(&self) -> Affine {
        self.transforms.last().copied().unwrap_or_default()
    }

    fn clip(&mut self, area: kurbo::Rect) {
        let area = self.transform().transform_rect_bbox(area);
        let area = match self.clips.last() {
            Some(outer) => outer.intersect(area),
            None => area,
        };
        self.clips.push(area);
    }

    fn check_colour(&mut self, palette_index: u16) {
        self.drawable &= palette_index == FOREGROUND || usize::from(palette_index) < self.colours;
    }

    fn check_stops(&mut self, stops: &[ColorStop]) {
        self.drawable &= !stops.is_empty();
        for stop in stops {
            self.check_colour(stop.palette_index);
            self.drawable &= stop.offset.is_finite();
        }
    }
}

impl ColorPainter for Survey<'_> {
    fn push_transform(&mut self, transform: Transform) {
        let local = Affine::new(
            [
                transform.xx,
                transform.yx,
                transform.xy,
                transform.yy,
                transform.dx,
                transform.dy,
            ]
            .map(f64::from),
        );
        self.drawable &= local.is_finite();
        self.transforms.push(self.transform() * local);
    }

    fn pop_transform(&mut self) {
        self.transforms.pop();
    }

    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        let mut pen = Pen::default();
        let settings = DrawSettings::unhinted(Size::unscaled(), LocationRef::new(self.coords));
        self.drawable &= self
            .font
            .outline_glyphs()
            .get(glyph_id)
            .is_some_and(|outline| outline.draw(settings, &mut pen).is_ok());
        self.clip(upright(pen.0.control_box()));
    }

    fn push_clip_box(&mut self, clip_box: BoundingBox<f32>) {
        self.clip(kurbo::Rect::new(
            f64::from(clip_box.x_min),
            f64::from(clip_box.y_min),
            f64::from(clip_box.x_max),
            f64::from(clip_box.y_max),
        ));
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn fill(&mut self, brush: Brush<'_>) {
        match brush {
            Brush::Solid { palette_index, .. } => self.check_colour(palette_index),
            Brush::LinearGradient { color_stops, .. }
            | Brush::RadialGradient { color_stops, .. }
            | Brush::SweepGradient { color_stops, .. } => self.check_stops(color_stops),
        }
        match self.clips.last() {
            Some(area) => {
                self.painted = Some(self.painted.map_or(*area, |painted| painted.union(*area)));
            }
            None => self.drawable = false,
        }
    }

    fn push_layer(&mut self, _: CompositeMode) {}
}

/// The area a colour glyph paints, in font units with y down, when all its layers can be drawn.
fn painted(font: &FontRef<'_>, run: &GlyphRun, id: GlyphId) -> Option<kurbo::Rect> {
    let coords: Vec<NormalizedCoord> = run
        .variation_coords()
        .iter()
        .map(|bits| NormalizedCoord::from_bits(*bits))
        .collect();
    let colours = font
        .cpal()
        .ok()
        .and_then(|palettes| palettes.color_records_array()?.ok())
        .map_or(0, <[_]>::len);
    let mut survey = Survey {
        font,
        coords: &coords,
        colours,
        transforms: Vec::new(),
        clips: Vec::new(),
        painted: None,
        drawable: true,
    };
    font.color_glyphs()
        .get(id)?
        .paint(LocationRef::new(&coords), &mut survey)
        .ok()?;
    let area = upright(survey.painted.filter(|_| survey.drawable)?);
    (area.is_finite() && area.area() > 0.0).then_some(area)
}

pub(crate) struct Painted {
    pub(crate) pixels: Vec<u8>,
    pub(crate) size: (u32, u32),
    pub(crate) area: kurbo::Rect,
}

/// A colour glyph's layers drawn at `scale` pixels to the point, around the glyph's origin.
pub(crate) fn paint(
    font: &FontRef<'_>,
    run: &GlyphRun,
    id: GlyphId,
    scale: f64,
) -> Option<Painted> {
    let units_per_em = f64::from(font.head().ok()?.units_per_em());
    let to_points = Affine::scale(f64::from(run.size) / units_per_em);
    let painted = to_points.transform_rect_bbox(painted(font, run, id)?);
    let longest = painted.width().max(painted.height()) * scale + 2.0;
    let scale = scale * (f64::from(LARGEST_BITMAP) / longest).min(1.0);
    let on_screen = Affine::scale(scale).transform_rect_bbox(painted).expand();
    let side = |length: f64| (length as u16).clamp(1, LARGEST_BITMAP as u16);
    let (width, height) = (side(on_screen.width()), side(on_screen.height()));
    let mut context = RenderContext::new(width, height);
    let mut resources = Resources::new();
    context.set_transform(Affine::translate((-on_screen.x0, -on_screen.y0)) * Affine::scale(scale));
    context.set_paint(draw::color(run.color));
    if let Err(error) = context
        .glyph_run(&mut resources, &run.font)
        .font_size(run.size)
        .hint(false)
        .normalized_coords(&run.coords)
        .fill_glyphs(std::iter::once(Glyph {
            id: id.to_u32(),
            x: 0.0,
            y: 0.0,
        }))
    {
        tracing::warn!("cannot draw a colour glyph: {error}");
        return None;
    }
    context.flush();
    let mut pixmap = Pixmap::new(width, height);
    context.render(&mut pixmap, &mut resources);
    Some(Painted {
        pixels: pixmap.data_as_u8_slice().to_vec(),
        size: (u32::from(width), u32::from(height)),
        area: Affine::scale(1.0 / scale).transform_rect_bbox(on_screen),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use bande_design::Color;
    use vello_cpu::peniko::{Blob, FontData};

    use super::*;
    use crate::cpu::CpuRenderer;
    use crate::scene::{Item, Scene};
    use crate::text::PositionedGlyph;

    const NO_STOPS: u32 = 1;
    const BLENDED: u32 = 2;
    const SOLID: u32 = 3;
    const BAD_COLOUR: u32 = 4;
    pub(crate) const GLYPHS: [u32; 4] = [NO_STOPS, BLENDED, SOLID, BAD_COLOUR];

    pub(crate) fn font() -> FontData {
        let bytes: &[u8] = include_bytes!("../testdata/layered-colour.ttf");
        FontData::new(Blob::from(bytes.to_vec()), 0)
    }

    pub(crate) fn run(font: &FontData, glyph: u32, size: f32) -> GlyphRun {
        let at = PositionedGlyph {
            id: glyph,
            x: 8.3,
            y: 40.0,
        };
        GlyphRun::new(font.clone(), size, Color::BLACK, vec![at])
    }

    #[test]
    fn layers_that_cannot_be_drawn_leave_a_plain_glyph_instead_of_panicking() {
        let font = font();
        let colours = GLYPHS.map(|glyph| {
            let scene = Scene {
                width: 64,
                height: 64,
                background: Color::WHITE,
                items: vec![Item::Glyphs(run(&font, glyph, 30.0))],
            };
            let drawn = CpuRenderer::new().render(&scene).expect("a frame");
            let coloured = |pixel: &[u8; 4]| pixel[0] != pixel[1] || pixel[1] != pixel[2];
            let pixels = drawn.rgba.as_chunks::<4>().0;
            (
                pixels.iter().any(|pixel| pixel[0] < 200 || pixel[1] < 200),
                pixels.iter().any(coloured),
            )
        });
        assert_eq!(
            colours,
            [(true, false), (true, true), (true, true), (true, false)]
        );
    }
}
