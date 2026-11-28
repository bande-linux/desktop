use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Weak};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{
    self, Affine, BezPath, ParamCurve as _, PathSeg, Shape as _, Stroke, StrokeOpts, Vec2,
};
use vello_cpu::peniko::{BlendMode, Compose, Extend, Gradient, ImageQuality, ImageSampler, Mix};
use vello_cpu::{ImageId, ImageSource, PaintType};

use crate::damage::Region;
use crate::outline::{GlyphOutline, GlyphOutlines, TOLERANCE};
use crate::scene::{
    Image, Item, Paint, Path, PathElement, Picture, Pixels, Point, Recorded, Rect, Shape, Stop,
    Transform,
};
use crate::surface::{SurfaceScaling, surface_area};
use crate::text::GlyphRun;
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
}

const TILE: f64 = 4.0;
const TILE_BYTES: f64 = 16.0;
const SMALL_TILES: f64 = 64.0;
const TILES_PER_LINE: f64 = 3.0;
const MOST_COVERAGE: f64 = 96.0 * 1024.0 * 1024.0;
const MOST_PAINTS: u32 = 1 << 20;
const MOST_GRADIENTS: u32 = 1 << 13;
const FARTHEST: f64 = 1e8;
const FARTHEST_TEXEL: f64 = 4e6;
const PLACEABLE: f64 = 32_000.0;
const RELATIVE_TOLERANCE: f64 = 1e-6;
const ANTIALIASING_PIXEL: f64 = 1.0;
const LYING_MARGIN: f64 = 2.0;
const SHADOW_DEVIATIONS: f64 = 2.5;

pub(crate) const CLEAR: BlendMode = BlendMode::new(Mix::Normal, Compose::Clear);

pub(crate) fn color(color: Color) -> AlphaColor<Srgb> {
    AlphaColor::from_rgba8(color.r, color.g, color.b, color.a)
}

fn coordinate(value: f32) -> f64 {
    match value.is_nan() {
        true => 0.0,
        false => f64::from(value).clamp(-FARTHEST, FARTHEST),
    }
}

fn point(point: Point) -> kurbo::Point {
    kurbo::Point::new(coordinate(point.x), coordinate(point.y))
}

fn place(value: f32) -> f64 {
    match value.is_nan() {
        true => 0.0,
        false => f64::from(value).clamp(-f64::from(f32::MAX), f64::from(f32::MAX)),
    }
}

fn spot(point: Point) -> kurbo::Point {
    kurbo::Point::new(place(point.x), place(point.y))
}

/// The polygons of a flattened `path` cut to `bounds`, each edge kept on its own line.
fn cut_to(path: &BezPath, bounds: kurbo::Rect) -> BezPath {
    let mut cut = BezPath::new();
    let mut polygon = Vec::new();
    let mut close = |polygon: &mut Vec<kurbo::Point>| {
        let kept = within(std::mem::take(polygon), bounds);
        if let Some((first, rest)) = kept.split_first() {
            cut.move_to(*first);
            rest.iter().for_each(|corner| cut.line_to(*corner));
            cut.close_path();
        }
    };
    kurbo::flatten(path, TOLERANCE, |element| match element {
        kurbo::PathEl::MoveTo(at) => {
            close(&mut polygon);
            polygon.push(at);
        }
        kurbo::PathEl::LineTo(at) => polygon.push(at),
        kurbo::PathEl::ClosePath => close(&mut polygon),
        kurbo::PathEl::QuadTo(..) | kurbo::PathEl::CurveTo(..) => {}
    });
    close(&mut polygon);
    cut
}

type Side = (fn(kurbo::Point) -> f64, f64, bool);

fn within(mut polygon: Vec<kurbo::Point>, bounds: kurbo::Rect) -> Vec<kurbo::Point> {
    let sides: [Side; 4] = [
        (|at| at.x, bounds.x0, true),
        (|at| at.x, bounds.x1, false),
        (|at| at.y, bounds.y0, true),
        (|at| at.y, bounds.y1, false),
    ];
    for (along, edge, above) in sides {
        let inside = |at: kurbo::Point| (along(at) >= edge) == above || along(at) == edge;
        let Some(&last) = polygon.last() else {
            return polygon;
        };
        let mut kept = Vec::with_capacity(polygon.len() + 2);
        let mut from = last;
        for to in polygon {
            if inside(to) != inside(from) {
                let share = (edge - along(from)) / (along(to) - along(from));
                kept.push(from.lerp(to, share));
            }
            if inside(to) {
                kept.push(to);
            }
            from = to;
        }
        polygon = kept;
    }
    polygon
}

fn tolerance(longest_curve: f64) -> f64 {
    TOLERANCE.max(longest_curve * RELATIVE_TOLERANCE)
}

fn longest_curve(path: &BezPath) -> f64 {
    path.segments()
        .filter(|segment| !matches!(segment, PathSeg::Line(_)))
        .map(|curve| {
            let curve = curve.to_cubic();
            let extent = kurbo::Rect::from_points(curve.p0, curve.p3)
                .union_pt(curve.p1)
                .union_pt(curve.p2);
            extent.width().max(extent.height())
        })
        .fold(0.0, f64::max)
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
        place(rect.x),
        place(rect.y),
        place(rect.x + rect.width),
        place(rect.y + rect.height),
    )
}

fn outline(shape: &Shape) -> BezPath {
    let rect = rect(shape.rect);
    if shape.radius > 0.0 {
        let shorter = rect.width().abs().min(rect.height().abs());
        rect.to_rounded_rect(f64::from(shape.radius))
            .to_path(tolerance(coordinate(shape.radius).min(shorter)))
    } else {
        rect.to_path(TOLERANCE)
    }
}

fn flattened(path: &BezPath) -> BezPath {
    let mut lines = BezPath::new();
    kurbo::flatten(path, tolerance(longest_curve(path)), |element| {
        lines.push(element);
    });
    lines
}

fn std_deviation(blur_radius: f32) -> f32 {
    (coordinate(blur_radius).max(0.0) / 2.0 * std::f64::consts::SQRT_2) as f32
}

fn shadowed(area: kurbo::Rect) -> kurbo::Rect {
    let bounds = kurbo::Rect::new(-FARTHEST, -FARTHEST, FARTHEST, FARTHEST);
    area.intersect(bounds)
}

fn path(shape: &Shape) -> BezPath {
    flattened(&outline(shape))
}

fn stroke_path(shape: &Shape, width: f32) -> BezPath {
    stroked(outline(shape), &Stroke::new(coordinate(width)))
}

fn stroked(path: BezPath, stroke: &Stroke) -> BezPath {
    let widened = tolerance(longest_curve(&path).max(stroke.width.abs()));
    flattened(&kurbo::stroke(
        path,
        stroke,
        &StrokeOpts::default(),
        widened,
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

    #[cfg(feature = "gpu")]
    pub(crate) fn len(&self) -> usize {
        self.0.len()
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
        glyphs: Vec<(Arc<GlyphOutline>, Vec2)>,
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
            PathElement::Move(to) => bezier.move_to(spot(to)),
            PathElement::Line(to) => bezier.line_to(spot(to)),
            PathElement::Cubic(first, second, to) => {
                bezier.curve_to(spot(first), spot(second), spot(to));
            }
            PathElement::Close => bezier.close_path(),
        }
    }
    bezier
}

fn snapped(base: Affine, offset: Point) -> Affine {
    let [scale, skew_y, skew_x, scale_y, x, y] = base.as_coeffs();
    if skew_x != 0.0 || skew_y != 0.0 || scale != scale_y {
        return base * Affine::translate((f64::from(offset.x), f64::from(offset.y)));
    }
    let device = |origin: f64, by: f32| (origin + scale * f64::from(by)).round();
    Affine::translate((device(x, offset.x), device(y, offset.y))) * Affine::scale(scale)
}

fn record_run(outlines: &mut GlyphOutlines, run: &GlyphRun, origin: Point, ops: &mut Vec<Op>) {
    let coloured = outlines.is_color_font(run);
    let mut plain = Vec::new();
    for glyph in &run.glyphs {
        let (x, y) = (origin.x + glyph.x, origin.y + glyph.y);
        let bitmap = match coloured {
            true => outlines.coloured(run, glyph.id),
            false => None,
        };
        match bitmap {
            Some(bitmap) => {
                let area = bitmap.placed(x, y, outlines.scale());
                let quality = ImageQuality::Medium;
                ops.push(image_op(area, area, &bitmap.image, Extend::Pad, quality));
            }
            None => plain.extend(
                outlines
                    .outline(run, glyph.id)
                    .map(|outline| (outline, Vec2::new(f64::from(x), f64::from(y)))),
            ),
        }
    }
    if !plain.is_empty() {
        ops.push(Op::Outlines {
            color: color(run.color),
            glyphs: plain,
        });
    }
}

fn image_quality(tile: Rect, image: &Image) -> ImageQuality {
    let native = tile.width == image.width() as f32
        && tile.height == image.height() as f32
        && tile.x.fract() == 0.0
        && tile.y.fract() == 0.0;
    match native {
        true => ImageQuality::Low,
        false => ImageQuality::High,
    }
}

fn image_op(
    area: kurbo::Rect,
    tile: kurbo::Rect,
    image: &Image,
    extend: Extend,
    quality: ImageQuality,
) -> Op {
    Op::Image {
        path: area.to_path(TOLERANCE),
        image: image.clone(),
        sampler: ImageSampler::default()
            .with_quality(quality)
            .with_extend(extend),
        placement: Affine::translate(tile.origin().to_vec2())
            * Affine::scale_non_uniform(
                tile.width() / f64::from(image.width()),
                tile.height() / f64::from(image.height()),
            ),
    }
}

fn drawable(area: kurbo::Rect, farthest: f64) -> bool {
    [area.x0, area.y0, area.x1, area.y1]
        .iter()
        .all(|edge| edge.abs() <= farthest)
}

#[derive(Clone, Copy)]
struct Reach<'a> {
    drawn: &'a [kurbo::Rect],
    transform: Affine,
}

impl Reach<'_> {
    fn through(self, transform: Affine) -> Self {
        Self { transform, ..self }
    }

    fn touches(self, bounds: kurbo::Rect) -> bool {
        let on_screen = self
            .transform
            .transform_rect_bbox(bounds)
            .inflate(ANTIALIASING_PIXEL, ANTIALIASING_PIXEL);
        on_screen.is_finite() && self.drawn.iter().any(|drawn| drawn.overlaps(on_screen))
    }

    fn lies_within(self, shape: &Shape, inset: f32) -> bool {
        let [scale_x, skew_y, skew_x, scale_y, _, _] = self.transform.as_coeffs();
        if skew_x != 0.0 || skew_y != 0.0 || scale_x <= 0.0 || scale_y <= 0.0 {
            return false;
        }
        let half = f64::from(shape.rect.width.min(shape.rect.height)) / 2.0;
        let inset = f64::from(inset);
        if inset >= half {
            return false;
        }
        let corner = (f64::from(shape.radius).min(half) - inset).max(0.0);
        let inside = rect(shape.rect).inflate(-inset, -inset);
        let part = |x: f64, y: f64| {
            self.transform
                .transform_rect_bbox(inside.inflate(-x, -y))
                .inflate(-LYING_MARGIN, -LYING_MARGIN)
        };
        let (wide, tall) = (part(0.0, corner), part(corner, 0.0));
        self.drawn
            .iter()
            .all(|drawn| wide.contains_rect(*drawn) || tall.contains_rect(*drawn))
    }

    fn outline(self) -> BezPath {
        let local = self.transform.inverse();
        let mut path = BezPath::new();
        for drawn in self.drawn {
            let reached = drawn.inflate(ANTIALIASING_PIXEL, ANTIALIASING_PIXEL);
            path.extend(local.transform_rect_bbox(reached).path_elements(TOLERANCE));
        }
        path
    }

    fn skips(self, item: &Item) -> bool {
        match item {
            Item::Fill { shape, .. } => !self.touches(rect(shape.rect)),
            Item::Clip { shape, items } => !self.touches(rect(shape.rect)) && !composites(items),
            Item::Stroke { shape, width, .. } => {
                let reach = f64::from(*width);
                !self.touches(rect(shape.rect).inflate(reach, reach))
                    || self.lies_within(shape, *width)
            }
            Item::CastShadow {
                shape,
                offset,
                blur_radius,
                ..
            } => {
                let reach = coordinate(*blur_radius * 4.0 + offset.x.abs() + offset.y.abs());
                !self.touches(rect(shape.rect).inflate(reach, reach))
                    || self.lies_within(shape, 0.0)
            }
            Item::Text { origin, paragraph } => !paragraph.ink().is_some_and(|ink| {
                let at = Vec2::new(f64::from(origin.x), f64::from(origin.y));
                self.touches(rect(ink) + at)
            }),
            Item::Image { rect: area, .. } | Item::Tiles { rect: area, .. } => {
                !self.touches(rect(*area))
            }
            Item::Picture(picture) => {
                !picture
                    .bounds()
                    .is_some_and(|bounds| self.touches(rect(bounds)))
                    && !composites(picture.items())
            }
            _ => false,
        }
    }
}

fn record_picture(picture: &Picture, caches: &mut Caches) {
    if caches.pictures.get(picture.id()).is_some() {
        return;
    }
    let mut ops = Vec::new();
    record(picture.items(), caches, &mut ops, None);
    caches
        .pictures
        .insert(picture.id(), ops.into(), picture.liveness());
}

fn composites(items: &[Item]) -> bool {
    items.iter().any(|item| match item {
        Item::Surface { .. } => true,
        Item::Clip { items, .. }
        | Item::Translate { items, .. }
        | Item::Opacity { items, .. }
        | Item::Transform { items, .. }
        | Item::Scale { items, .. } => composites(items),
        Item::Picture(picture) => composites(picture.items()),
        _ => false,
    })
}

fn outline_within(shape: &Shape, reach: Option<Reach<'_>>) -> BezPath {
    match reach.filter(|reach| reach.lies_within(shape, 0.0)) {
        Some(reach) => reach.outline(),
        None => path(shape),
    }
}

fn record(items: &[Item], caches: &mut Caches, ops: &mut Vec<Op>, reach: Option<Reach<'_>>) {
    for item in items {
        if !reach.is_some_and(|reach| reach.skips(item)) {
            record_item(item, caches, ops, reach);
        }
    }
}

fn scoped(
    opening: Op,
    items: &[Item],
    caches: &mut Caches,
    ops: &mut Vec<Op>,
    reach: Option<Reach<'_>>,
) {
    ops.push(opening);
    record(items, caches, ops, reach);
    ops.push(Op::End);
}

fn record_item(item: &Item, caches: &mut Caches, ops: &mut Vec<Op>, reach: Option<Reach<'_>>) {
    match item {
        Item::Fill { shape, paint: fill } => ops.push(Op::Fill {
            path: outline_within(shape, reach),
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
            rect: shadowed(rect(shape.rect)) + point(*offset).to_vec2(),
            radius: coordinate(shape.radius) as f32,
            std_dev: std_deviation(*blur_radius),
            color: color(*shadow),
        }),
        Item::CastShadow {
            shape,
            offset,
            blur_radius,
            color: shadow,
        } => {
            let reach = coordinate(*blur_radius * 4.0 + offset.x.abs() + offset.y.abs());
            let around = rect(shape.rect).inflate(reach, reach);
            let mut outside = kurbo::Shape::to_path(&around, TOLERANCE);
            outside.extend(path(shape).reverse_subpaths().iter());
            ops.push(Op::Clip(outside));
            ops.push(Op::Shadow {
                rect: shadowed(rect(shape.rect)) + point(*offset).to_vec2(),
                radius: coordinate(shape.radius) as f32,
                std_dev: std_deviation(*blur_radius),
                color: color(*shadow),
            });
            ops.push(Op::End);
        }
        Item::Text { origin, paragraph } => {
            for run in paragraph.glyph_runs() {
                record_run(&mut caches.outlines, &run, *origin, ops);
            }
        }
        Item::Glyphs(run) => record_run(&mut caches.outlines, run, Point::default(), ops),
        Item::Clip { shape, items } => {
            let clip = outline_within(shape, reach);
            scoped(Op::Clip(clip), items, caches, ops, reach);
        }
        Item::Translate { offset, items } => {
            let offset = Point::new(coordinate(offset.x) as f32, coordinate(offset.y) as f32);
            let reach = reach.map(|reach| reach.through(snapped(reach.transform, offset)));
            scoped(Op::Snap(offset), items, caches, ops, reach);
        }
        Item::Opacity { alpha, items } => {
            let layer = Op::Layer {
                blend: BlendMode::default(),
                alpha: coordinate(*alpha).clamp(0.0, 1.0) as f32,
            };
            scoped(layer, items, caches, ops, reach);
        }
        Item::Transform { transform, items } => {
            let local = affine(*transform);
            if !local.is_finite() {
                return;
            }
            let reach = reach.map(|reach| reach.through(reach.transform * local));
            scoped(Op::Transform(local), items, caches, ops, reach);
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
            if !scaled.is_finite() {
                return;
            }
            let reach = reach.map(|reach| reach.through(reach.transform * scaled));
            scoped(Op::Transform(scaled), items, caches, ops, reach);
        }
        Item::Line {
            from,
            to,
            width,
            paint: stroke,
        } => ops.push(Op::Fill {
            path: stroked(
                kurbo::Line::new(spot(*from), spot(*to)).to_path(TOLERANCE),
                &Stroke::new(coordinate(*width)).with_caps(kurbo::Cap::Round),
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
                &Stroke::new(coordinate(*width))
                    .with_caps(kurbo::Cap::Round)
                    .with_join(kurbo::Join::Round),
            ),
            paint: paint(stroke),
        }),
        Item::Image { rect: area, image } => {
            let quality = image_quality(*area, image);
            ops.push(image_op(
                rect(*area),
                rect(*area),
                image,
                Extend::Pad,
                quality,
            ));
        }
        Item::Tiles {
            rect: area,
            tile,
            image,
        } => {
            let quality = image_quality(*tile, image);
            ops.push(image_op(
                rect(*area),
                rect(*tile),
                image,
                Extend::Repeat,
                quality,
            ));
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
        Item::Picture(picture) => {
            record_picture(picture, caches);
            ops.push(Op::Picture(picture.clone()));
        }
    }
}

fn region_outline(regions: &[Region]) -> BezPath {
    let mut path = BezPath::new();
    for region in regions {
        let (left, top) = (f64::from(region.x), f64::from(region.y));
        let (right, bottom) = (
            left + f64::from(region.width),
            top + f64::from(region.height),
        );
        path.move_to((left, top));
        path.line_to((right, top));
        path.line_to((right, bottom));
        path.line_to((left, bottom));
        path.close_path();
    }
    path
}

fn region_rect(region: &Region) -> kurbo::Rect {
    kurbo::Rect::new(
        f64::from(region.x),
        f64::from(region.y),
        f64::from(region.x + region.width),
        f64::from(region.y + region.height),
    )
}

pub(crate) struct Frame {
    pub(crate) scale: f64,
    pub(crate) origin: (u32, u32),
    pub(crate) size: (u16, u16),
}

pub(crate) struct Recording {
    ops: Vec<Op>,
    drawn: Vec<kurbo::Rect>,
    base: Affine,
    clip: Option<BezPath>,
}

fn gather_images(ops: &[Op], caches: &Caches, found: &mut Vec<Image>) {
    for op in ops {
        match op {
            Op::Image { image, .. } => found.push(image.clone()),
            Op::Picture(picture) => {
                if let Some(ops) = caches.pictures.get(picture.id()) {
                    gather_images(&ops, caches, found);
                }
            }
            _ => {}
        }
    }
}

impl Recording {
    pub(crate) fn new(
        caches: &mut Caches,
        items: &[Item],
        frame: &Frame,
        damage: Option<&[Region]>,
    ) -> Self {
        caches.pictures.take_dropped();
        if caches.outlines.rescale(frame.scale) {
            caches.pictures = Retained::default();
        }
        let viewport = kurbo::Rect::new(0.0, 0.0, f64::from(frame.size.0), f64::from(frame.size.1));
        let base = Affine::translate((-f64::from(frame.origin.0), -f64::from(frame.origin.1)))
            * Affine::scale(frame.scale);
        let drawn = match damage {
            Some(regions) => regions.iter().map(region_rect).collect(),
            None => vec![viewport],
        };
        let mut ops = Vec::new();
        let reach = Reach {
            drawn: &drawn,
            transform: base,
        };
        record(items, caches, &mut ops, Some(reach));
        Self {
            ops,
            drawn,
            base,
            clip: damage.map(region_outline),
        }
    }

    pub(crate) fn images(&self, caches: &Caches) -> Vec<Image> {
        let mut found = Vec::new();
        gather_images(&self.ops, caches, &mut found);
        found
    }

    pub(crate) fn replay(&self, canvas: &mut impl Canvas, caches: &mut Caches) {
        let mut encoder = Encoder {
            canvas,
            caches,
            drawn: &self.drawn,
            clips: Vec::new(),
            isolated: false,
            budget: Budget {
                coverage: MOST_COVERAGE,
                paints: MOST_PAINTS,
                gradients: MOST_GRADIENTS,
            },
        };
        if let Some(clip) = &self.clip {
            encoder.canvas.set_transform(Affine::IDENTITY);
            encoder.canvas.push_clip_path(clip);
        }
        encoder.replay(&self.ops, self.base);
        if self.clip.is_some() {
            encoder.canvas.pop_clip();
        }
        encoder.caches.outlines.end_frame();
    }
}

struct Budget {
    coverage: f64,
    paints: u32,
    gradients: u32,
}

impl Budget {
    fn affords(&mut self, path: &BezPath, transform: Affine, drawn: &[kurbo::Rect]) -> bool {
        let on_screen = transform.transform_rect_bbox(path.control_box());
        let tiles: f64 = drawn
            .iter()
            .filter(|drawn| drawn.overlaps(on_screen))
            .map(|drawn| tiles_covered(path, transform, drawn.intersect(on_screen)))
            .sum();
        self.coverage -= tiles * TILE_BYTES;
        self.coverage >= 0.0
    }

    fn affords_paint(&mut self, paint: &PaintType) -> bool {
        match paint {
            PaintType::Solid(_) => true,
            PaintType::Gradient(_) => spend(&mut self.gradients) && spend(&mut self.paints),
            PaintType::Image(_) => spend(&mut self.paints),
        }
    }
}

fn spend(left: &mut u32) -> bool {
    let remaining = left.checked_sub(1);
    *left = remaining.unwrap_or_default();
    remaining.is_some()
}

fn tiles_covered(path: &BezPath, transform: Affine, shown: kurbo::Rect) -> f64 {
    let inside = (shown.width() / TILE + 2.0) * (shown.height() / TILE + 2.0);
    if inside <= SMALL_TILES {
        return inside;
    }
    let along: f64 = path
        .segments()
        .map(|line| {
            let line = kurbo::Rect::from_points(transform * line.start(), transform * line.end());
            if line.x0 > shown.x1 || line.y0 > shown.y1 || line.y1 < shown.y0 {
                return 0.0;
            }
            let crossed = line.intersect(shown);
            (crossed.width() + crossed.height()) / TILE + TILES_PER_LINE
        })
        .sum();
    inside.min(along)
}

enum Scope {
    Clip,
    Layer { isolated: bool },
    Transform(Affine),
}

struct Encoder<'a, C> {
    canvas: &'a mut C,
    caches: &'a mut Caches,
    drawn: &'a [kurbo::Rect],
    clips: Vec<(BezPath, Affine)>,
    isolated: bool,
    budget: Budget,
}

impl<C: Canvas> Encoder<'_, C> {
    fn reaches(&self, bounds: kurbo::Rect, transform: Affine) -> bool {
        Reach {
            drawn: self.drawn,
            transform,
        }
        .touches(bounds)
    }

    /// `path` cut to where the renderer can place it, unchanged where it is shown.
    fn placeable<'p>(path: &'p BezPath, transform: Affine) -> Option<Cow<'p, BezPath>> {
        let reach = PLACEABLE;
        if drawable(transform.transform_rect_bbox(path.control_box()), reach) {
            return Some(Cow::Borrowed(path));
        }
        if transform.determinant() == 0.0 {
            return None;
        }
        let bounds = kurbo::Rect::new(-reach, -reach, reach, reach);
        let mut cut = cut_to(&(transform * path), bounds);
        cut.apply_affine(transform.inverse());
        Some(Cow::Owned(cut))
    }

    fn samples(&self, path: &BezPath, transform: Affine, placement: Affine) -> bool {
        let to_texels = (transform * placement).inverse();
        let on_screen = transform.transform_rect_bbox(path.control_box());
        to_texels
            .as_coeffs()
            .iter()
            .all(|coefficient| coefficient.abs() <= FARTHEST_TEXEL)
            && self
                .drawn
                .iter()
                .filter(|drawn| drawn.overlaps(on_screen))
                .all(|drawn| {
                    let shown = drawn.intersect(on_screen);
                    drawable(to_texels.transform_rect_bbox(shown), FARTHEST_TEXEL)
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
                    let Some(path) = Self::placeable(path, transform) else {
                        continue;
                    };
                    if self.reaches(path.control_box(), transform)
                        && self.budget.affords(&path, transform, self.drawn)
                        && self.budget.affords_paint(paint)
                    {
                        self.canvas.set_paint(paint.clone());
                        self.canvas.fill_path(&path);
                    }
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
                    let Some(path) = Self::placeable(path, transform) else {
                        continue;
                    };
                    let path = path.as_ref();
                    if !self.reaches(path.control_box(), transform)
                        || !self.samples(path, transform, *placement)
                        || !self.budget.affords(path, transform, self.drawn)
                        || !spend(&mut self.budget.paints)
                    {
                        continue;
                    }
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
                    let reach = f64::from(*std_dev) * SHADOW_DEVIATIONS;
                    let blurred = rect.inflate(reach, reach);
                    let edge = blurred.to_path(TOLERANCE);
                    if !self.reaches(blurred, transform)
                        || !self.budget.affords(&edge, transform, self.drawn)
                        || !spend(&mut self.budget.paints)
                    {
                        continue;
                    }
                    self.canvas.set_paint((*color).into());
                    self.canvas
                        .fill_blurred_rounded_rect(rect, *radius, *std_dev);
                }
                Op::Outlines { color, glyphs } => {
                    self.canvas.set_paint((*color).into());
                    for (outline, at) in glyphs {
                        let placed = transform * Affine::translate(*at);
                        if self.reaches(outline.bounds, placed)
                            && self.budget.affords(&outline.path, placed, self.drawn)
                        {
                            self.canvas.set_transform(placed);
                            self.canvas.fill_path(&outline.path);
                        }
                    }
                    self.canvas.set_transform(transform);
                }
                Op::Clip(clip) => {
                    let clip = match Self::placeable(clip, transform) {
                        Some(clip) if self.budget.affords(&clip, transform, self.drawn) => {
                            clip.into_owned()
                        }
                        _ => BezPath::new(),
                    };
                    self.canvas.push_clip_layer(&clip);
                    self.clips.push((clip, transform));
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
                    let backing = rect(*frame).to_path(TOLERANCE);
                    let Some(backing) = Self::placeable(&backing, transform) else {
                        continue;
                    };
                    self.canvas.set_paint(color(Color::BLACK).into());
                    self.canvas.fill_path(&backing);
                    let scale = transform.as_coeffs()[0];
                    if let Some(area) = surface_area(*frame, *size, *scaling, scale) {
                        self.punch(&path(&Shape::rect(area)), transform);
                    }
                }
                Op::Picture(picture) => {
                    let reached = picture
                        .bounds()
                        .is_some_and(|bounds| self.reaches(rect(bounds), transform));
                    if (reached || composites(picture.items()))
                        && let Some(ops) = self.caches.pictures.get(picture.id())
                    {
                        self.replay(&ops, transform);
                        self.canvas.set_transform(transform);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Scene;
    use crate::text::{TextAlign, TextStyle, TextSystem};

    #[derive(Default)]
    struct Counted(usize);

    impl Canvas for Counted {
        fn set_paint(&mut self, _: PaintType) {}
        fn set_transform(&mut self, _: Affine) {}
        fn set_paint_transform(&mut self, _: Affine) {}
        fn reset_paint_transform(&mut self) {}
        fn fill_path(&mut self, _: &BezPath) {
            self.0 += 1;
        }
        fn fill_blurred_rounded_rect(&mut self, _: &kurbo::Rect, _: f32, _: f32) {}
        fn push_clip_layer(&mut self, _: &BezPath) {}
        fn push_layer(&mut self, _: BlendMode, _: f32) {}
        fn pop_layer(&mut self) {}
        fn push_clip_path(&mut self, _: &BezPath) {}
        fn pop_clip(&mut self) {}
        fn clear_path(&mut self, _: &BezPath) {}
    }

    #[test]
    fn a_changed_line_encodes_only_the_glyphs_near_it() {
        const LINES: u16 = 50;
        const COLUMNS: usize = 100;
        const LINE_HEIGHT: u16 = 20;
        let mut text = TextSystem::new();
        let style = TextStyle::new(13.0, Color::BLACK).monospace();
        let paragraph = text.paragraph(&"m".repeat(COLUMNS), style, None, TextAlign::Start);
        let items: Vec<Item> = (0..LINES)
            .map(|line| Item::Text {
                origin: Point::new(0.0, f32::from(line * LINE_HEIGHT)),
                paragraph: paragraph.clone(),
            })
            .collect();
        let changed = Region {
            x: 0,
            y: u32::from(10 * LINE_HEIGHT),
            width: 2000,
            height: u32::from(LINE_HEIGHT),
        };
        let mut canvas = Counted::default();
        let frame = Frame {
            scale: 1.0,
            origin: (0, 0),
            size: (2000, LINES * LINE_HEIGHT),
        };
        let mut caches = Caches::default();
        Recording::new(&mut caches, &items, &frame, Some(&[changed]))
            .replay(&mut canvas, &mut caches);
        assert!(
            (COLUMNS..=3 * COLUMNS).contains(&canvas.0),
            "{} glyphs were encoded for one line",
            canvas.0
        );
    }

    #[test]
    fn no_more_is_encoded_than_the_renderer_has_room_for() {
        let mut text = TextSystem::new();
        let style = TextStyle::new(200.0, Color::BLACK);
        let paragraph = text.paragraph("m", style, None, TextAlign::Start);
        let items: Vec<Item> = std::iter::repeat_n(paragraph, 200_000)
            .map(|paragraph| Item::Text {
                origin: Point::new(0.0, 0.0),
                paragraph,
            })
            .collect();
        let mut canvas = Counted::default();
        let frame = Frame {
            scale: 1.0,
            origin: (0, 0),
            size: (2000, 1000),
        };
        let mut caches = Caches::default();
        Recording::new(&mut caches, &items, &frame, None).replay(&mut canvas, &mut caches);
        assert!(
            (1_000..items.len()).contains(&canvas.0),
            "{} glyphs were encoded",
            canvas.0
        );
    }

    #[test]
    fn lines_running_far_past_the_window_cost_only_what_it_shows_of_them() {
        let lines: Vec<Item> = (0..2000)
            .map(|line| {
                let x = line as f32;
                Item::Line {
                    from: Point::new(x - 1e6, -1e6),
                    to: Point::new(x + 1e6, 1e6),
                    width: 1.0,
                    paint: Color::BLACK.into(),
                }
            })
            .collect();
        let mut canvas = Counted::default();
        let frame = Frame {
            scale: 1.0,
            origin: (0, 0),
            size: (2000, 1000),
        };
        let mut caches = Caches::default();
        Recording::new(&mut caches, &lines, &frame, None).replay(&mut canvas, &mut caches);
        assert_eq!(canvas.0, lines.len());
    }

    #[test]
    fn only_the_images_a_frame_shows_are_handed_to_the_atlases() {
        let image = || Image::from_rgba(30, 20, vec![255; 2400]).expect("an image");
        let (shown, elsewhere) = (image(), image());
        let items = [(&shown, 10.0), (&elsewhere, 5000.0)].map(|(image, x)| Item::Image {
            rect: Rect::new(x, 10.0, 30.0, 20.0),
            image: image.clone(),
        });
        let frame = Frame {
            scale: 1.0,
            origin: (0, 0),
            size: (200, 100),
        };
        let mut caches = Caches::default();
        let recording = Recording::new(&mut caches, &items, &frame, None);
        let handed: Vec<u64> = recording.images(&caches).iter().map(Image::id).collect();
        assert_eq!(handed, [shown.id()]);
    }

    #[test]
    fn small_curves_of_huge_shapes_stay_as_round() {
        let corner = |far: f32| {
            let shape = |inset: f32| Shape::rounded(Rect::new(inset, inset, far, far), 24.0);
            let scene = Scene {
                width: 48,
                height: 48,
                background: Color::WHITE,
                items: vec![
                    Item::Fill {
                        shape: shape(12.0),
                        paint: Color::BLACK.into(),
                    },
                    Item::Stroke {
                        shape: shape(6.0),
                        width: 2.0,
                        paint: Color::hex(0x2020d0).into(),
                    },
                ],
            };
            crate::cpu::CpuRenderer::new()
                .render(&scene)
                .expect("a frame")
                .rgba
        };
        let (small, huge) = (corner(100.0), corner(1e7));
        let differs = small
            .iter()
            .zip(&huge)
            .map(|(one, other)| one.abs_diff(*other));
        assert!(differs.max().is_some_and(|differs| differs <= 2));
    }

    #[test]
    fn what_runs_far_past_the_window_keeps_its_direction() {
        let black: Paint = Color::BLACK.into();
        let far = Path::new()
            .move_to(Point::new(0.0, 0.0))
            .line_to(Point::new(2e9, 1e9))
            .line_to(Point::new(0.0, 1e9))
            .close();
        let scene = |item: Item| Scene {
            width: 64,
            height: 64,
            background: Color::WHITE,
            items: vec![item],
        };
        let line = scene(Item::Line {
            from: Point::new(0.0, 0.0),
            to: Point::new(1e9, 5e8),
            width: 4.0,
            paint: black.clone(),
        });
        let filled = scene(Item::FillPath {
            path: far,
            paint: black,
        });
        let mut renderer = crate::cpu::CpuRenderer::new();
        let shade = |scene: &Scene, renderer: &mut crate::cpu::CpuRenderer| {
            let drawn = renderer.render(scene).expect("a frame").rgba;
            move |x: usize, y: usize| drawn[(y * 64 + x) * 4]
        };
        let along = shade(&line, &mut renderer);
        assert_eq!((along(40, 20), along(40, 40)), (0, 255));
        let inside = shade(&filled, &mut renderer);
        assert_eq!((inside(40, 30), inside(40, 10)), (0, 255));
    }

    #[test]
    fn absurd_sizes_and_places_are_drawn_or_left_out() {
        let mut text = TextSystem::new();
        let image = Image::from_rgba(30, 20, vec![255; 2400]).expect("an image");
        let mut items = Vec::new();
        for value in [1e30, -1e30, 1e9, f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
            let shapes = [
                Shape::rounded(Rect::new(0.0, 0.0, value, value), value),
                Shape::rounded(Rect::new(value, value, 40.0, 40.0), 9.0),
            ];
            for shape in shapes {
                let black: Paint = Color::BLACK.into();
                items.push(Item::Fill {
                    shape,
                    paint: black.clone(),
                });
                items.push(Item::Stroke {
                    shape,
                    width: value,
                    paint: black.clone(),
                });
                items.push(Item::Shadow {
                    shape,
                    offset: Point::new(value, value),
                    blur_radius: value,
                    color: Color::BLACK,
                });
                items.push(Item::Clip {
                    shape,
                    items: vec![Item::Opacity {
                        alpha: value,
                        items: vec![Item::Scale {
                            origin: Point::new(value, 0.0),
                            factor: value,
                            items: vec![Item::Line {
                                from: Point::new(0.0, value),
                                to: Point::new(value, 1.0),
                                width: value,
                                paint: black,
                            }],
                        }],
                    }],
                });
            }
            let style = TextStyle::new(value, Color::BLACK);
            items.push(Item::Text {
                origin: Point::new(0.0, 0.0),
                paragraph: text.paragraph("absurd", style, Some(100.0), TextAlign::Start),
            });
        }
        for width in [1e-30, 1e-12, 1e-7, 1e-4, 1.0] {
            for (x, y) in [(0.0, 0.0), (-1e7, 3.0), (2.5, 1e7), (-1e30, -1e30)] {
                for rect in [
                    Rect::new(-9.0, -9.0, 99.0, 99.0),
                    Rect::new(5.0, 5.0, 1e-6, 40.0),
                ] {
                    items.push(Item::Tiles {
                        rect,
                        tile: Rect::new(x, y, width, width * 2.0),
                        image: image.clone(),
                    });
                }
            }
        }
        let scene = |width, height| Scene {
            width,
            height,
            background: Color::WHITE,
            items: items.clone(),
        };
        let mut renderer = crate::cpu::CpuRenderer::new();
        assert!(renderer.render(&scene(64, 64)).is_ok());
        let endless = Scene {
            items: vec![Item::Fill {
                shape: Shape::rect(Rect::new(-1e30, -1e30, 2e30, f32::INFINITY)),
                paint: Color::BLACK.into(),
            }],
            ..scene(4, 4)
        };
        let mut filled = vec![0; 8 * 8 * 4];
        assert!(renderer.render_into(&endless, 2.0, &mut filled).is_ok());
        assert_eq!(&filled[..4], [0, 0, 0, 255]);
        assert!(renderer.render(&scene(u32::from(u16::MAX), 1)).is_err());
        assert!(crate::damage::damage(&scene(64, 64), &scene(64, 65), 1.0).is_none());
    }
}
