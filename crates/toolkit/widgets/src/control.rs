use unbound_graphics::{Color, Image, Item, Palette, Path, Point, Radius, Rect, Shape};

use super::Context;

const RING_WIDTH: f32 = 2.0;
const RING_GAP: f32 = 1.0;
const FIELD_OUTLINE: f32 = 1.5;
const FIELD_RING_OPACITY: f32 = 0.35;
const CHEVRON_WIDTH: f32 = 4.0;
const CHEVRON_HEIGHT: f32 = 2.25;
const GLYPH_STROKE: f32 = 1.5;

pub fn focus_ring(cx: &Context, shape: Shape, items: &mut Vec<Item>) {
    focus_ring_for(&cx.palette, shape, items);
}

pub fn focus_ring_for(palette: &Palette, shape: Shape, items: &mut Vec<Item>) {
    ring(palette.focus, shape, items);
}

fn ring(color: Color, shape: Shape, items: &mut Vec<Item>) {
    let offset = RING_GAP + RING_WIDTH / 2.0;
    items.push(Item::Stroke {
        shape: Shape::rounded(shape.rect.inset(-offset), shape.radius + offset),
        width: RING_WIDTH,
        paint: color.into(),
    });
}

pub(crate) fn field_focus(cx: &Context, shape: Shape, items: &mut Vec<Item>) {
    let inset = FIELD_OUTLINE / 2.0;
    items.push(Item::Stroke {
        shape: Shape::rounded(shape.rect.inset(inset), shape.radius - inset),
        width: FIELD_OUTLINE,
        paint: cx.palette.focus.into(),
    });
    ring(
        cx.palette.focus.with_opacity(FIELD_RING_OPACITY),
        shape,
        items,
    );
}

pub(crate) fn outline(shape: Shape, color: Color, items: &mut Vec<Item>) {
    items.push(Item::Stroke {
        shape: Shape::rounded(shape.rect.inset(0.5), shape.radius - 0.5),
        width: 1.0,
        paint: color.into(),
    });
}

pub(crate) fn chevron_down(centre: Point, color: Color, items: &mut Vec<Item>) {
    items.push(Item::StrokePath {
        path: Path::new()
            .move_to(Point::new(
                centre.x - CHEVRON_WIDTH,
                centre.y - CHEVRON_HEIGHT,
            ))
            .line_to(Point::new(centre.x, centre.y + CHEVRON_HEIGHT))
            .line_to(Point::new(
                centre.x + CHEVRON_WIDTH,
                centre.y - CHEVRON_HEIGHT,
            )),
        width: GLYPH_STROKE,
        paint: color.into(),
    });
}

pub(crate) fn glyph_line(from: Point, to: Point, color: Color, items: &mut Vec<Item>) {
    items.push(Item::Line {
        from,
        to,
        width: GLYPH_STROKE,
        paint: color.into(),
    });
}

pub(crate) struct Symbol {
    source: Image,
    tinted: Option<(Color, Image)>,
}

impl Symbol {
    pub(crate) fn new(source: Image) -> Self {
        Self {
            source,
            tinted: None,
        }
    }

    pub(crate) fn tint(&mut self, color: Color) {
        if self.tinted.as_ref().is_some_and(|(tint, _)| *tint == color) {
            return;
        }
        self.tinted = self.source.tinted(color).map(|image| (color, image));
    }

    pub(crate) fn paint(&self, rect: Rect, items: &mut Vec<Item>) {
        let image = self
            .tinted
            .as_ref()
            .map_or(&self.source, |(_, image)| image);
        items.push(Item::Image {
            rect,
            image: image.clone(),
        });
    }
}

pub(crate) fn square_capped(points: &[Point], width: f32) -> Path {
    let half = width / 2.0;
    let direction = |from: Point, to: Point| {
        let (dx, dy) = (to.x - from.x, to.y - from.y);
        let length = dx.hypot(dy).max(f32::EPSILON);
        (dx / length, dy / length)
    };
    let segments: Vec<(f32, f32)> = points
        .windows(2)
        .map(|pair| direction(pair[0], pair[1]))
        .collect();
    let (Some(first), Some(last)) = (segments.first(), segments.last()) else {
        return Path::new();
    };
    let normal = |(dx, dy): (f32, f32)| (-dy, dx);
    let offset = |side: f32| {
        points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let before = segments[index.saturating_sub(1)];
                let after = segments[index.min(segments.len() - 1)];
                let (a, b) = (normal(before), normal(after));
                let dot = a.0 * b.0 + a.1 * b.1;
                let miter = ((a.0 + b.0) / (1.0 + dot), (a.1 + b.1) / (1.0 + dot));
                let cap = match index {
                    0 => (-first.0 * half, -first.1 * half),
                    index if index == points.len() - 1 => (last.0 * half, last.1 * half),
                    _ => (0.0, 0.0),
                };
                Point::new(
                    point.x + cap.0 + side * miter.0 * half,
                    point.y + cap.1 + side * miter.1 * half,
                )
            })
            .collect::<Vec<_>>()
    };
    let left = offset(1.0);
    let right = offset(-1.0);
    let outline: Vec<Point> = left.into_iter().chain(right.into_iter().rev()).collect();
    Path::through(&outline).close()
}

pub(crate) fn paint_field(cx: &Context, frame: Rect, focused: bool, items: &mut Vec<Item>) {
    let shape = Shape::rounded(frame, Radius::S);
    items.push(Item::Fill {
        shape,
        paint: cx.palette.field.into(),
    });
    match focused {
        true => field_focus(cx, shape, items),
        false => outline(shape, cx.palette.edge, items),
    }
}
