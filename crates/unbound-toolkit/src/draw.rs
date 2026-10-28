use std::collections::HashMap;
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{self, Affine, BezPath, Shape as _, Stroke, StrokeOpts};
use vello_cpu::peniko::Gradient;

use vello_cpu::peniko::{ImageQuality, ImageSampler};
use vello_cpu::{Glyph, ImageId, ImageSource, PaintType};

use crate::outline::{GlyphOutlines, TOLERANCE};
use crate::scene::{Image, Item, Paint, Shape};
use crate::text::GlyphRun;
use crate::tokens::Color;

pub(crate) trait Canvas {
    fn set_paint(&mut self, paint: PaintType);
    fn set_transform(&mut self, transform: Affine);
    fn set_paint_transform(&mut self, transform: Affine);
    fn reset_paint_transform(&mut self);
    fn fill_path(&mut self, path: &BezPath);
    fn fill_blurred_rounded_rect(&mut self, rect: &kurbo::Rect, radius: f32, std_dev: f32);
    fn push_clip_layer(&mut self, path: &BezPath);
    fn pop_layer(&mut self);
    fn fill_glyphs(&mut self, run: &GlyphRun, glyphs: &[Glyph]);
}

pub(crate) fn color(color: Color) -> AlphaColor<Srgb> {
    AlphaColor::from_rgba8(color.r, color.g, color.b, color.a)
}

fn paint(paint: &Paint) -> PaintType {
    match paint {
        Paint::Solid(solid) => color(*solid).into(),
        Paint::LinearGradient { start, end, stops } => {
            let stops: Vec<(f32, AlphaColor<Srgb>)> = stops
                .iter()
                .map(|stop| (stop.offset, color(stop.color)))
                .collect();
            Gradient::new_linear(
                (f64::from(start.x), f64::from(start.y)),
                (f64::from(end.x), f64::from(end.y)),
            )
            .with_stops(stops.as_slice())
            .into()
        }
    }
}

fn rect(shape: &Shape) -> kurbo::Rect {
    let rect = shape.rect;
    kurbo::Rect::new(
        f64::from(rect.x),
        f64::from(rect.y),
        f64::from(rect.x + rect.width),
        f64::from(rect.y + rect.height),
    )
}

fn outline(shape: &Shape) -> BezPath {
    let rect = rect(shape);
    if shape.radius > 0.0 {
        rect.to_rounded_rect(f64::from(shape.radius))
            .to_path(TOLERANCE)
    } else {
        rect.to_path(TOLERANCE)
    }
}

fn flattened(path: &BezPath) -> BezPath {
    let mut lines = BezPath::new();
    kurbo::flatten(path, TOLERANCE, |element| lines.push(element));
    lines
}

fn path(shape: &Shape) -> BezPath {
    flattened(&outline(shape))
}

fn stroke_path(shape: &Shape, width: f32) -> BezPath {
    flattened(&kurbo::stroke(
        outline(shape),
        &Stroke::new(f64::from(width)),
        &StrokeOpts::default(),
        TOLERANCE,
    ))
}

pub(crate) type ImageIds = HashMap<u64, ImageId>;

pub(crate) fn images(items: &[Item], found: &mut Vec<Image>) {
    for item in items {
        match item {
            Item::Image { image, .. } => found.push(image.clone()),
            Item::Clip { items, .. } => images(items, found),
            _ => {}
        }
    }
}

pub(crate) fn encode(
    canvas: &mut impl Canvas,
    outlines: &mut GlyphOutlines,
    image_ids: &ImageIds,
    items: &[Item],
) {
    for item in items {
        match item {
            Item::Fill { shape, paint: fill } => {
                canvas.set_paint(paint(fill));
                canvas.fill_path(&path(shape));
            }
            Item::Stroke {
                shape,
                width,
                paint: stroke,
            } => {
                canvas.set_paint(paint(stroke));
                canvas.fill_path(&stroke_path(shape, *width));
            }
            Item::Shadow {
                shape,
                offset,
                blur_radius,
                color: shadow,
            } => {
                canvas.set_paint(color(*shadow).into());
                let rect = rect(shape) + kurbo::Vec2::new(f64::from(offset.x), f64::from(offset.y));
                canvas.fill_blurred_rounded_rect(
                    &rect,
                    shape.radius,
                    blur_radius / 2.0 * std::f32::consts::SQRT_2,
                );
            }
            Item::Text { origin, paragraph } => {
                for run in paragraph.glyph_runs() {
                    canvas.set_paint(color(run.color).into());
                    if outlines.is_color_font(&run) {
                        let glyphs: Vec<Glyph> = run
                            .glyphs
                            .iter()
                            .map(|glyph| Glyph {
                                id: glyph.id,
                                x: origin.x + glyph.x,
                                y: origin.y + glyph.y,
                            })
                            .collect();
                        canvas.fill_glyphs(&run, &glyphs);
                        continue;
                    }
                    for glyph in &run.glyphs {
                        let Some(outline) = outlines.outline(&run, glyph.id) else {
                            continue;
                        };
                        canvas.set_transform(Affine::translate((
                            f64::from(origin.x + glyph.x),
                            f64::from(origin.y + glyph.y),
                        )));
                        canvas.fill_path(&outline);
                    }
                    canvas.set_transform(Affine::IDENTITY);
                }
            }
            Item::Clip { shape, items } => {
                canvas.push_clip_layer(&path(shape));
                encode(canvas, outlines, image_ids, items);
                canvas.pop_layer();
            }
            Item::Image { rect: area, image } => {
                let Some(id) = image_ids.get(&image.id()) else {
                    continue;
                };
                let shape = Shape::rect(*area);
                let native = area.width == image.width() as f32
                    && area.height == image.height() as f32
                    && area.x.fract() == 0.0
                    && area.y.fract() == 0.0;
                let quality = if native {
                    ImageQuality::Low
                } else {
                    ImageQuality::High
                };
                canvas.set_paint(PaintType::Image(vello_cpu::Image {
                    image: ImageSource::opaque_id(*id),
                    sampler: ImageSampler::default().with_quality(quality),
                }));
                canvas.set_paint_transform(
                    Affine::translate((f64::from(area.x), f64::from(area.y)))
                        * Affine::scale_non_uniform(
                            f64::from(area.width) / f64::from(image.width()),
                            f64::from(area.height) / f64::from(image.height()),
                        ),
                );
                canvas.fill_path(&path(&shape));
                canvas.reset_paint_transform();
            }
        }
    }
}
