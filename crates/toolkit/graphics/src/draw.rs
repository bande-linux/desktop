use std::collections::HashMap;
use std::sync::{Arc, Weak};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{self, Affine, BezPath, Shape as _, Stroke, StrokeOpts, Vec2};
use vello_cpu::peniko::{BlendMode, Compose, Extend, Gradient, ImageQuality, ImageSampler, Mix};
use vello_cpu::{Glyph, ImageId, ImageSource, PaintType};

use crate::outline::{GlyphOutlines, TOLERANCE};
use crate::scene::{
    Image, Item, Paint, Path, PathElement, Picture, Pixels, Point, Recorded, Rect, Shape, Stop,
    Transform,
};
use crate::surface::{SurfaceScaling, surface_area};
use crate::text::{GlyphRun, PositionedGlyph};
use bande_design::Color;

pub(crate) trait Canvas {
    fn set_paint(&mut self, paint: PaintType);
    fn set_transform(&mut self, transform: Affine);
    fn set_paint_transform(&mut self, transform: Affine);
    fn reset_paint_transform(&mut self);
    fn fill_path(&mut self, path: &BezPath);
    fn fill_blurred_rounded_rect(&mut self, rect: &kurbo::Rect, radius: f32, std_dev: f32);
    fn push_clip_layer(&mut self, path: &BezPath);
    fn push_layer(&mut self, blend: BlendMode, alpha: f32);
    fn pop_layer(&mut self);
    fn push_clip_path(&mut self, path: &BezPath);
    fn pop_clip(&mut self);
    fn clear_path(&mut self, path: &BezPath);
    fn fill_glyphs(&mut self, run: &GlyphRun, glyphs: &[Glyph]);
}

pub(crate) const CLEAR: BlendMode = BlendMode::new(Mix::Normal, Compose::Clear);

pub(crate) fn color(color: Color) -> AlphaColor<Srgb> {
    AlphaColor::from_rgba8(color.r, color.g, color.b, color.a)
}

fn point(point: Point) -> kurbo::Point {
    kurbo::Point::new(f64::from(point.x), f64::from(point.y))
}

fn gradient(gradient: Gradient, stops: &[Stop]) -> PaintType {
    let stops: Vec<(f32, AlphaColor<Srgb>)> = stops
        .iter()
        .map(|stop| (stop.offset, color(stop.color)))
        .collect();
    gradient.with_stops(stops.as_slice()).into()
}

fn paint(paint: &Paint) -> PaintType {
    match paint {
        Paint::Solid(solid) => color(*solid).into(),
        Paint::LinearGradient { start, end, stops } => {
            gradient(Gradient::new_linear(point(*start), point(*end)), stops)
        }
        Paint::RadialGradient {
            center,
            radius,
            stops,
        } => gradient(Gradient::new_radial(point(*center), *radius), stops),
        Paint::ConicGradient {
            center,
            start_angle,
            end_angle,
            stops,
        } => gradient(
            Gradient::new_sweep(point(*center), *start_angle, *end_angle),
            stops,
        ),
    }
}

fn affine(transform: Transform) -> Affine {
    Affine::new(transform.matrix.map(f64::from))
}

fn rect(rect: Rect) -> kurbo::Rect {
    kurbo::Rect::new(
        f64::from(rect.x),
        f64::from(rect.y),
        f64::from(rect.x + rect.width),
        f64::from(rect.y + rect.height),
    )
}

fn outline(shape: &Shape) -> BezPath {
    let rect = rect(shape.rect);
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

fn stroked(path: impl IntoIterator<Item = kurbo::PathEl>, stroke: &Stroke) -> BezPath {
    flattened(&kurbo::stroke(
        path,
        stroke,
        &StrokeOpts::default(),
        TOLERANCE,
    ))
}

pub(crate) struct Retained<V, T>(HashMap<u64, (V, Weak<T>)>);

impl<V, T> Default for Retained<V, T> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}

impl<V: Clone, T> Retained<V, T> {
    pub(crate) fn get(&self, id: u64) -> Option<V> {
        self.0.get(&id).map(|(value, _)| value.clone())
    }

    pub(crate) fn insert(&mut self, id: u64, value: V, liveness: Weak<T>) {
        self.0.insert(id, (value, liveness));
    }

    pub(crate) fn take_dropped(&mut self) -> Vec<V> {
        let mut dropped = Vec::new();
        self.0.retain(|_, (value, alive)| {
            let kept = alive.strong_count() > 0;
            if !kept {
                dropped.push(value.clone());
            }
            kept
        });
        dropped
    }
}

#[derive(Default)]
pub(crate) struct Caches {
    pub(crate) outlines: GlyphOutlines,
    pub(crate) images: Retained<ImageId, Pixels>,
    pictures: Retained<Arc<[Op]>, Recorded>,
}

pub(crate) fn images(items: &[Item], found: &mut Vec<Image>) {
    for item in items {
        match item {
            Item::Image { image, .. } | Item::Tiles { image, .. } => found.push(image.clone()),
            Item::Clip { items, .. }
            | Item::Translate { items, .. }
            | Item::Opacity { items, .. }
            | Item::Blend { items, .. }
            | Item::Transform { items, .. }
            | Item::Scale { items, .. } => images(items, found),
            Item::Picture(picture) => images(picture.items(), found),
            _ => {}
        }
    }
}

enum Op {
    Fill {
        path: BezPath,
        paint: PaintType,
    },
    Image {
        path: BezPath,
        image: Image,
        sampler: ImageSampler,
        placement: Affine,
    },
    Shadow {
        rect: kurbo::Rect,
        radius: f32,
        std_dev: f32,
        color: AlphaColor<Srgb>,
    },
    Outlines {
        color: AlphaColor<Srgb>,
        glyphs: Vec<(Arc<BezPath>, Vec2)>,
    },
    Glyphs {
        run: GlyphRun,
        glyphs: Vec<Glyph>,
    },
    Clip(BezPath),
    Layer {
        blend: BlendMode,
        alpha: f32,
    },
    Snap(Point),
    Transform(Affine),
    End,
    Surface {
        frame: Rect,
        size: (u32, u32),
        scaling: SurfaceScaling,
    },
    Picture(Picture),
}

fn bezier(path: &Path) -> BezPath {
    let mut bezier = BezPath::new();
    for element in &path.elements {
        match *element {
            PathElement::Move(to) => bezier.move_to(point(to)),
            PathElement::Line(to) => bezier.line_to(point(to)),
            PathElement::Cubic(first, second, to) => {
                bezier.curve_to(point(first), point(second), point(to));
            }
            PathElement::Close => bezier.close_path(),
        }
    }
    bezier
}

fn snapped(base: Affine, offset: Point) -> Affine {
    let [scale, _, _, _, x, y] = base.as_coeffs();
    let device = |origin: f64, by: f32| (origin + scale * f64::from(by)).round();
    Affine::translate((device(x, offset.x), device(y, offset.y))) * Affine::scale(scale)
}

fn record_run(outlines: &mut GlyphOutlines, run: &GlyphRun, origin: Point, ops: &mut Vec<Op>) {
    let at = |glyph: &PositionedGlyph| (origin.x + glyph.x, origin.y + glyph.y);
    if outlines.is_color_font(run) {
        let glyphs = run
            .glyphs
            .iter()
            .map(|glyph| {
                let (x, y) = at(glyph);
                Glyph { id: glyph.id, x, y }
            })
            .collect();
        ops.push(Op::Glyphs {
            run: run.clone(),
            glyphs,
        });
        return;
    }
    let glyphs = run
        .glyphs
        .iter()
        .filter_map(|glyph| {
            let (x, y) = at(glyph);
            Some((
                outlines.outline(run, glyph.id)?,
                Vec2::new(f64::from(x), f64::from(y)),
            ))
        })
        .collect();
    ops.push(Op::Outlines {
        color: color(run.color),
        glyphs,
    });
}

fn image_op(area: Rect, tile: Rect, image: &Image, extend: Extend) -> Op {
    let native = tile.width == image.width() as f32
        && tile.height == image.height() as f32
        && tile.x.fract() == 0.0
        && tile.y.fract() == 0.0;
    let quality = if native {
        ImageQuality::Low
    } else {
        ImageQuality::High
    };
    Op::Image {
        path: path(&Shape::rect(area)),
        image: image.clone(),
        sampler: ImageSampler::default()
            .with_quality(quality)
            .with_extend(extend),
        placement: Affine::translate((f64::from(tile.x), f64::from(tile.y)))
            * Affine::scale_non_uniform(
                f64::from(tile.width) / f64::from(image.width()),
                f64::from(tile.height) / f64::from(image.height()),
            ),
    }
}

fn record(items: &[Item], outlines: &mut GlyphOutlines, ops: &mut Vec<Op>) {
    for item in items {
        record_item(item, outlines, ops);
    }
}

fn scoped(opening: Op, items: &[Item], outlines: &mut GlyphOutlines, ops: &mut Vec<Op>) {
    ops.push(opening);
    record(items, outlines, ops);
    ops.push(Op::End);
}

fn record_item(item: &Item, outlines: &mut GlyphOutlines, ops: &mut Vec<Op>) {
    match item {
        Item::Fill { shape, paint: fill } => ops.push(Op::Fill {
            path: path(shape),
            paint: paint(fill),
        }),
        Item::Stroke {
            shape,
            width,
            paint: stroke,
        } => ops.push(Op::Fill {
            path: stroke_path(shape, *width),
            paint: paint(stroke),
        }),
        Item::Shadow {
            shape,
            offset,
            blur_radius,
            color: shadow,
        } => ops.push(Op::Shadow {
            rect: rect(shape.rect) + Vec2::new(f64::from(offset.x), f64::from(offset.y)),
            radius: shape.radius,
            std_dev: blur_radius / 2.0 * std::f32::consts::SQRT_2,
            color: color(*shadow),
        }),
        Item::CastShadow {
            shape,
            offset,
            blur_radius,
            color: shadow,
        } => {
            let reach = f64::from(*blur_radius * 4.0 + offset.x.abs() + offset.y.abs());
            let around = rect(shape.rect).inflate(reach, reach);
            let mut outside = kurbo::Shape::to_path(&around, TOLERANCE);
            outside.extend(path(shape).reverse_subpaths().iter());
            ops.push(Op::Clip(outside));
            ops.push(Op::Shadow {
                rect: rect(shape.rect) + Vec2::new(f64::from(offset.x), f64::from(offset.y)),
                radius: shape.radius,
                std_dev: blur_radius / 2.0 * std::f32::consts::SQRT_2,
                color: color(*shadow),
            });
            ops.push(Op::End);
        }
        Item::Text { origin, paragraph } => {
            for run in paragraph.glyph_runs() {
                record_run(outlines, &run, *origin, ops);
            }
        }
        Item::Glyphs(run) => record_run(outlines, run, Point::default(), ops),
        Item::Clip { shape, items } => scoped(Op::Clip(path(shape)), items, outlines, ops),
        Item::Translate { offset, items } => scoped(Op::Snap(*offset), items, outlines, ops),
        Item::Opacity { alpha, items } => {
            let layer = Op::Layer {
                blend: BlendMode::default(),
                alpha: *alpha,
            };
            scoped(layer, items, outlines, ops);
        }
        Item::Blend { mode, items } => {
            let layer = Op::Layer {
                blend: (*mode).into(),
                alpha: 1.0,
            };
            scoped(layer, items, outlines, ops);
        }
        Item::Transform { transform, items } => {
            scoped(Op::Transform(affine(*transform)), items, outlines, ops);
        }
        Item::Scale {
            origin,
            factor,
            items,
        } => {
            let pivot = point(*origin).to_vec2();
            let scaled = Affine::translate(pivot)
                * Affine::scale(f64::from(*factor))
                * Affine::translate(-pivot);
            scoped(Op::Transform(scaled), items, outlines, ops);
        }
        Item::Line {
            from,
            to,
            width,
            paint: stroke,
        } => ops.push(Op::Fill {
            path: stroked(
                kurbo::Line::new(point(*from), point(*to)).path_elements(TOLERANCE),
                &Stroke::new(f64::from(*width)).with_caps(kurbo::Cap::Round),
            ),
            paint: paint(stroke),
        }),
        Item::FillPath { path, paint: fill } => ops.push(Op::Fill {
            path: flattened(&bezier(path)),
            paint: paint(fill),
        }),
        Item::StrokePath {
            path,
            width,
            paint: stroke,
        } => ops.push(Op::Fill {
            path: stroked(
                bezier(path),
                &Stroke::new(f64::from(*width))
                    .with_caps(kurbo::Cap::Round)
                    .with_join(kurbo::Join::Round),
            ),
            paint: paint(stroke),
        }),
        Item::Image { rect, image } => ops.push(image_op(*rect, *rect, image, Extend::Pad)),
        Item::Tiles { rect, tile, image } => {
            ops.push(image_op(*rect, *tile, image, Extend::Repeat));
        }
        Item::Surface {
            frame,
            size,
            scaling,
            ..
        } => ops.push(Op::Surface {
            frame: *frame,
            size: *size,
            scaling: *scaling,
        }),
        Item::Picture(picture) => ops.push(Op::Picture(picture.clone())),
    }
}

pub(crate) struct Frame {
    pub(crate) scale: f64,
    pub(crate) origin: (u32, u32),
    pub(crate) size: (u16, u16),
}

pub(crate) fn encode(
    canvas: &mut impl Canvas,
    caches: &mut Caches,
    items: &[Item],
    frame: &Frame,
    outer_clip: Option<BezPath>,
) {
    caches.pictures.take_dropped();
    let mut ops = Vec::new();
    record(items, &mut caches.outlines, &mut ops);
    let base = Affine::translate((-f64::from(frame.origin.0), -f64::from(frame.origin.1)))
        * Affine::scale(frame.scale);
    let mut encoder = Encoder {
        canvas,
        caches,
        viewport: kurbo::Rect::new(0.0, 0.0, f64::from(frame.size.0), f64::from(frame.size.1)),
        clips: Vec::new(),
        isolated: false,
    };
    if let Some(clip) = outer_clip {
        encoder.canvas.set_transform(Affine::IDENTITY);
        encoder.canvas.push_clip_layer(&clip);
        encoder.clips.push((clip, Affine::IDENTITY));
    }
    encoder.replay(&ops, base);
    if !encoder.clips.is_empty() {
        encoder.canvas.pop_layer();
    }
    encoder.caches.outlines.end_frame();
}

enum Scope {
    Clip,
    Layer { isolated: bool },
    Transform(Affine),
}

struct Encoder<'a, C> {
    canvas: &'a mut C,
    caches: &'a mut Caches,
    viewport: kurbo::Rect,
    clips: Vec<(BezPath, Affine)>,
    isolated: bool,
}

impl<C: Canvas> Encoder<'_, C> {
    fn recording(&mut self, picture: &Picture) -> Arc<[Op]> {
        if let Some(ops) = self.caches.pictures.get(picture.id()) {
            return ops;
        }
        let mut ops = Vec::new();
        record(picture.items(), &mut self.caches.outlines, &mut ops);
        let ops: Arc<[Op]> = ops.into();
        self.caches
            .pictures
            .insert(picture.id(), Arc::clone(&ops), picture.liveness());
        ops
    }

    fn visible(&self, bounds: Option<Rect>, transform: Affine) -> bool {
        bounds.is_some_and(|bounds| {
            !transform
                .transform_rect_bbox(rect(bounds))
                .intersect(self.viewport)
                .is_zero_area()
        })
    }

    fn punch(&mut self, hole: &BezPath, base: Affine) {
        let reach = !self.isolated;
        if reach {
            for _ in &self.clips {
                self.canvas.pop_layer();
            }
            for (clip, at) in &self.clips {
                self.canvas.set_transform(*at);
                self.canvas.push_clip_path(clip);
            }
        }
        self.canvas.set_transform(base);
        self.canvas.clear_path(hole);
        if reach {
            for _ in &self.clips {
                self.canvas.pop_clip();
            }
            for (clip, at) in &self.clips {
                self.canvas.set_transform(*at);
                self.canvas.push_clip_layer(clip);
            }
            self.canvas.set_transform(base);
        }
    }

    fn replay(&mut self, ops: &[Op], mut transform: Affine) {
        let mut scopes = Vec::new();
        self.canvas.set_transform(transform);
        for op in ops {
            match op {
                Op::Fill { path, paint } => {
                    self.canvas.set_paint(paint.clone());
                    self.canvas.fill_path(path);
                }
                Op::Image {
                    path,
                    image,
                    sampler,
                    placement,
                } => {
                    let Some(id) = self.caches.images.get(image.id()) else {
                        continue;
                    };
                    self.canvas.set_paint(PaintType::Image(vello_cpu::Image {
                        image: ImageSource::opaque_id(id),
                        sampler: *sampler,
                    }));
                    self.canvas.set_paint_transform(*placement);
                    self.canvas.fill_path(path);
                    self.canvas.reset_paint_transform();
                }
                Op::Shadow {
                    rect,
                    radius,
                    std_dev,
                    color,
                } => {
                    self.canvas.set_paint((*color).into());
                    self.canvas
                        .fill_blurred_rounded_rect(rect, *radius, *std_dev);
                }
                Op::Outlines { color, glyphs } => {
                    self.canvas.set_paint((*color).into());
                    for (outline, at) in glyphs {
                        self.canvas
                            .set_transform(transform * Affine::translate(*at));
                        self.canvas.fill_path(outline);
                    }
                    self.canvas.set_transform(transform);
                }
                Op::Glyphs { run, glyphs } => {
                    self.canvas.set_paint(color(run.color).into());
                    self.canvas.fill_glyphs(run, glyphs);
                }
                Op::Clip(clip) => {
                    self.canvas.push_clip_layer(clip);
                    self.clips.push((clip.clone(), transform));
                    scopes.push(Scope::Clip);
                }
                Op::Layer { blend, alpha } => {
                    self.canvas.push_layer(*blend, *alpha);
                    scopes.push(Scope::Layer {
                        isolated: std::mem::replace(&mut self.isolated, true),
                    });
                }
                Op::Snap(offset) => {
                    scopes.push(Scope::Transform(transform));
                    transform = snapped(transform, *offset);
                    self.canvas.set_transform(transform);
                }
                Op::Transform(local) => {
                    scopes.push(Scope::Transform(transform));
                    transform *= *local;
                    self.canvas.set_transform(transform);
                }
                Op::End => match scopes.pop() {
                    Some(Scope::Clip) => {
                        self.clips.pop();
                        self.canvas.pop_layer();
                    }
                    Some(Scope::Layer { isolated }) => {
                        self.isolated = isolated;
                        self.canvas.pop_layer();
                    }
                    Some(Scope::Transform(outer)) => {
                        transform = outer;
                        self.canvas.set_transform(transform);
                    }
                    None => {}
                },
                Op::Surface {
                    frame,
                    size,
                    scaling,
                } => {
                    self.canvas.set_paint(color(Color::BLACK).into());
                    self.canvas.fill_path(&path(&Shape::rect(*frame)));
                    let scale = transform.as_coeffs()[0];
                    if let Some(area) = surface_area(*frame, *size, *scaling, scale) {
                        self.punch(&path(&Shape::rect(area)), transform);
                    }
                }
                Op::Picture(picture) => {
                    if self.visible(picture.bounds(), transform) {
                        let ops = self.recording(picture);
                        self.replay(&ops, transform);
                        self.canvas.set_transform(transform);
                    }
                }
            }
        }
    }
}
