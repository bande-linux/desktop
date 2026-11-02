use std::sync::atomic::{AtomicU64, Ordering};

use crate::scene::{Item, Point, Rect, Scene};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceId(u64);

impl SurfaceId {
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SurfaceScaling {
    #[default]
    Fit,
    Native,
}

pub fn surface_area(
    frame: Rect,
    size: (u32, u32),
    scaling: SurfaceScaling,
    scale: f64,
) -> Option<Rect> {
    let (width, height) = (size.0 as f32, size.1 as f32);
    if width < 1.0 || height < 1.0 || frame.width < 1.0 || frame.height < 1.0 {
        return None;
    }
    let fit = (frame.width / width).min(frame.height / height);
    let factor = match scaling {
        SurfaceScaling::Fit => fit,
        SurfaceScaling::Native => fit.min(1.0 / scale as f32),
    };
    let shown = (
        (width * factor).floor().max(1.0),
        (height * factor).floor().max(1.0),
    );
    Some(Rect::new(
        (frame.x + (frame.width - shown.0) / 2.0).round(),
        (frame.y + (frame.height - shown.1) / 2.0).round(),
        shown.0,
        shown.1,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedSurface {
    pub id: SurfaceId,
    pub area: Rect,
}

#[derive(Clone, Copy)]
struct Mapping {
    factor: f32,
    offset: Point,
    clip: Option<Rect>,
}

impl Mapping {
    fn rect(self, rect: Rect) -> Rect {
        Rect::new(
            self.offset.x + rect.x * self.factor,
            self.offset.y + rect.y * self.factor,
            rect.width * self.factor,
            rect.height * self.factor,
        )
    }

    fn clipped(self, rect: Rect) -> Option<Rect> {
        let rect = self.rect(rect);
        let Some(clip) = self.clip else {
            return Some(rect);
        };
        let left = rect.x.max(clip.x);
        let top = rect.y.max(clip.y);
        let right = (rect.x + rect.width).min(clip.x + clip.width);
        let bottom = (rect.y + rect.height).min(clip.y + clip.height);
        (right > left && bottom > top).then(|| Rect::new(left, top, right - left, bottom - top))
    }
}

pub fn placed_surfaces(scene: &Scene, scale: f64) -> Vec<PlacedSurface> {
    let mut placed = Vec::new();
    let identity = Mapping {
        factor: 1.0,
        offset: Point::default(),
        clip: None,
    };
    walk(&scene.items, identity, scale, &mut placed);
    placed
}

fn walk(items: &[Item], mapping: Mapping, scale: f64, placed: &mut Vec<PlacedSurface>) {
    for item in items {
        match item {
            Item::Surface {
                id,
                frame,
                size,
                scaling,
            } => {
                let area = surface_area(*frame, *size, *scaling, scale)
                    .filter(|area| mapping.clipped(*area).is_some());
                if let Some(area) = area {
                    placed.push(PlacedSurface {
                        id: *id,
                        area: mapping.rect(area),
                    });
                }
            }
            Item::Clip { shape, items } => {
                if let Some(clip) = mapping.clipped(shape.rect) {
                    let inner = Mapping {
                        clip: Some(clip),
                        ..mapping
                    };
                    walk(items, inner, scale, placed);
                }
            }
            Item::Translate { offset, items } => {
                let inner = Mapping {
                    offset: Point::new(
                        mapping.offset.x + offset.x * mapping.factor,
                        mapping.offset.y + offset.y * mapping.factor,
                    ),
                    ..mapping
                };
                walk(items, inner, scale, placed);
            }
            Item::Scale {
                origin,
                factor,
                items,
            } => {
                let inner = Mapping {
                    factor: mapping.factor * factor,
                    offset: Point::new(
                        mapping.offset.x + mapping.factor * origin.x * (1.0 - factor),
                        mapping.offset.y + mapping.factor * origin.y * (1.0 - factor),
                    ),
                    ..mapping
                };
                walk(items, inner, scale, placed);
            }
            Item::Transform { transform, items } => {
                let [factor, skew_y, skew_x, same, x, y] = transform.matrix;
                if skew_x == 0.0 && skew_y == 0.0 && factor == same && factor > 0.0 {
                    let inner = Mapping {
                        factor: mapping.factor * factor,
                        offset: Point::new(
                            mapping.offset.x + mapping.factor * x,
                            mapping.offset.y + mapping.factor * y,
                        ),
                        ..mapping
                    };
                    walk(items, inner, scale, placed);
                }
            }
            Item::Opacity { items, .. } | Item::Blend { items, .. } => {
                walk(items, mapping, scale, placed);
            }
            Item::Picture(picture) => walk(picture.items(), mapping, scale, placed),
            _ => {}
        }
    }
}
