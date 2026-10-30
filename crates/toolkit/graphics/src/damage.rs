use crate::scene::{Item, Path, PathElement, Point, Rect, Scene, Shape};

const MAX_REGIONS: usize = 8;
const ANTIALIASING_MARGIN: f32 = 1.0;
const SHADOW_REACH: f32 = 2.5;
const GLYPH_OVERHANG: f32 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    pub fn area(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self {
            x,
            y,
            width: (self.x + self.width).max(other.x + other.width) - x,
            height: (self.y + self.height).max(other.y + other.height) - y,
        }
    }

    fn touches(&self, other: &Self) -> bool {
        self.x <= other.x + other.width
            && other.x <= self.x + self.width
            && self.y <= other.y + other.height
            && other.y <= self.y + self.height
    }
}

pub fn damage(previous: &Scene, next: &Scene, scale: f64) -> Option<Vec<Region>> {
    if previous.width != next.width
        || previous.height != next.height
        || previous.background != next.background
    {
        return None;
    }
    let mut rects = Vec::new();
    diff(&previous.items, &next.items, &mut rects);
    let size = (
        crate::render::pixels(next.width, scale),
        crate::render::pixels(next.height, scale),
    );
    let mut regions: Vec<Region> = rects
        .into_iter()
        .filter_map(|rect| region(rect, scale, size))
        .collect();
    merge(&mut regions);
    let whole = u64::from(size.0) * u64::from(size.1);
    let damaged: u64 = regions.iter().map(Region::area).sum();
    (damaged * 2 <= whole).then_some(regions)
}

fn merge(regions: &mut Vec<Region>) {
    let mut merged = true;
    while merged {
        merged = false;
        'outer: for first in 0..regions.len() {
            for second in first + 1..regions.len() {
                if regions[first].touches(&regions[second]) {
                    let joined = regions[first].union(regions.swap_remove(second));
                    regions[first] = joined;
                    merged = true;
                    break 'outer;
                }
            }
        }
    }
    if regions.len() > MAX_REGIONS
        && let Some(joined) = regions.iter().copied().reduce(Region::union)
    {
        *regions = vec![joined];
    }
}

fn region(rect: Rect, scale: f64, size: (u32, u32)) -> Option<Region> {
    let rect = outset(rect, ANTIALIASING_MARGIN);
    let left = (f64::from(rect.x) * scale).floor().max(0.0) as u32;
    let top = (f64::from(rect.y) * scale).floor().max(0.0) as u32;
    let right = ((f64::from(rect.x + rect.width) * scale).ceil().max(0.0) as u32).min(size.0);
    let bottom = ((f64::from(rect.y + rect.height) * scale).ceil().max(0.0) as u32).min(size.1);
    (right > left && bottom > top).then_some(Region {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

fn diff(previous: &[Item], next: &[Item], damaged: &mut Vec<Rect>) {
    let prefix = previous
        .iter()
        .zip(next)
        .take_while(|(before, after)| before == after)
        .count();
    let (previous, next) = (&previous[prefix..], &next[prefix..]);
    let suffix = previous
        .iter()
        .rev()
        .zip(next.iter().rev())
        .take_while(|(before, after)| before == after)
        .count();
    let previous = &previous[..previous.len() - suffix];
    let next = &next[..next.len() - suffix];
    if previous.len() != next.len() {
        damaged.extend(previous.iter().chain(next).filter_map(bounds));
        return;
    }
    for (before, after) in previous.iter().zip(next) {
        if before != after {
            diff_item(before, after, damaged);
        }
    }
}

fn diff_item(before: &Item, after: &Item, damaged: &mut Vec<Rect>) {
    let mut inner = Vec::new();
    match (before, after) {
        (
            Item::Clip {
                shape: clip,
                items: previous,
            },
            Item::Clip {
                shape: same,
                items: next,
            },
        ) if clip == same => {
            diff(previous, next, &mut inner);
            damaged.extend(
                inner
                    .into_iter()
                    .filter_map(|rect| intersect(rect, clip.rect)),
            );
        }
        (
            Item::Translate {
                offset,
                items: previous,
            },
            Item::Translate {
                offset: same,
                items: next,
            },
        ) if offset == same => {
            diff(previous, next, &mut inner);
            damaged.extend(inner.into_iter().map(|rect| translated(rect, *offset)));
        }
        (
            Item::Opacity {
                alpha,
                items: previous,
            },
            Item::Opacity {
                alpha: same,
                items: next,
            },
        ) if alpha == same => diff(previous, next, damaged),
        (
            Item::Scale {
                origin,
                factor,
                items: previous,
            },
            Item::Scale {
                origin: same_origin,
                factor: same_factor,
                items: next,
            },
        ) if origin == same_origin && factor == same_factor => {
            diff(previous, next, &mut inner);
            damaged.extend(inner.into_iter().map(|rect| scaled(rect, *origin, *factor)));
        }
        _ => damaged.extend([bounds(before), bounds(after)].into_iter().flatten()),
    }
}

fn outset(rect: Rect, amount: f32) -> Rect {
    Rect::new(
        rect.x - amount,
        rect.y - amount,
        rect.width + 2.0 * amount,
        rect.height + 2.0 * amount,
    )
}

fn union(first: Rect, second: Rect) -> Rect {
    let left = first.x.min(second.x);
    let top = first.y.min(second.y);
    let right = (first.x + first.width).max(second.x + second.width);
    let bottom = (first.y + first.height).max(second.y + second.height);
    Rect::new(left, top, right - left, bottom - top)
}

fn intersect(first: Rect, second: Rect) -> Option<Rect> {
    let left = first.x.max(second.x);
    let top = first.y.max(second.y);
    let right = (first.x + first.width).min(second.x + second.width);
    let bottom = (first.y + first.height).min(second.y + second.height);
    (right > left && bottom > top).then(|| Rect::new(left, top, right - left, bottom - top))
}

fn translated(rect: Rect, offset: Point) -> Rect {
    outset(
        Rect::new(
            rect.x + offset.x,
            rect.y + offset.y,
            rect.width,
            rect.height,
        ),
        ANTIALIASING_MARGIN,
    )
}

fn scaled(rect: Rect, origin: Point, factor: f32) -> Rect {
    let x = origin.x + (rect.x - origin.x) * factor;
    let y = origin.y + (rect.y - origin.y) * factor;
    let (width, height) = (rect.width * factor, rect.height * factor);
    Rect::new(
        x.min(x + width),
        y.min(y + height),
        width.abs(),
        height.abs(),
    )
}

fn through(points: impl IntoIterator<Item = Point>) -> Option<Rect> {
    points
        .into_iter()
        .map(|point| Rect::new(point.x, point.y, 0.0, 0.0))
        .reduce(union)
}

fn path_bounds(path: &Path) -> Option<Rect> {
    through(path.elements.iter().flat_map(|element| match *element {
        PathElement::Move(point) | PathElement::Line(point) => vec![point],
        PathElement::Cubic(first, second, end) => vec![first, second, end],
        PathElement::Close => Vec::new(),
    }))
}

fn children(items: &[Item]) -> Option<Rect> {
    items.iter().filter_map(bounds).reduce(union)
}

fn stroked(shape: &Shape, width: f32) -> Rect {
    outset(shape.rect, width / 2.0)
}

fn bounds(item: &Item) -> Option<Rect> {
    match item {
        Item::Fill { shape, .. } => Some(shape.rect),
        Item::Stroke { shape, width, .. } => Some(stroked(shape, *width)),
        Item::Shadow {
            shape,
            offset,
            blur_radius,
            ..
        } => Some(outset(
            translated(shape.rect, *offset),
            blur_radius * SHADOW_REACH,
        )),
        Item::Text { origin, paragraph } => {
            let size = paragraph
                .glyph_runs()
                .iter()
                .map(|run| run.size)
                .fold(0.0, f32::max);
            Some(outset(
                Rect::new(origin.x, origin.y, paragraph.width(), paragraph.height()),
                size * GLYPH_OVERHANG,
            ))
        }
        Item::Clip { shape, items } => intersect(children(items)?, shape.rect),
        Item::Translate { offset, items } => Some(translated(children(items)?, *offset)),
        Item::Opacity { items, .. } => children(items),
        Item::Scale {
            origin,
            factor,
            items,
        } => Some(scaled(children(items)?, *origin, *factor)),
        Item::Line {
            from, to, width, ..
        } => Some(outset(through([*from, *to])?, *width)),
        Item::FillPath { path, .. } => path_bounds(path),
        Item::StrokePath { path, width, .. } => Some(outset(path_bounds(path)?, *width)),
        Item::Image { rect, .. } => Some(*rect),
    }
}

#[cfg(test)]
mod tests {
    use super::{Region, damage};
    use crate::scene::{Item, Point, Rect, Scene, Shape};
    use crate::tokens::Color;

    fn fill(x: f32, color: u32) -> Item {
        Item::Fill {
            shape: Shape::rect(Rect::new(x, 10.0, 20.0, 20.0)),
            paint: Color::hex(color).into(),
        }
    }

    fn scene(items: Vec<Item>) -> Scene {
        Scene {
            width: 400,
            height: 300,
            background: Color::hex(0xFFFFFF),
            items,
        }
    }

    fn icons(hovered: Option<usize>) -> Vec<Item> {
        let mut items = vec![fill(0.0, 0x336699)];
        for index in 0..4 {
            let x = 40.0 + 60.0 * index as f32;
            if hovered == Some(index) {
                items.push(fill(x - 2.0, 0xDDDDDD));
            }
            items.push(fill(x, 0x112233));
        }
        items
    }

    #[test]
    fn an_unchanged_scene_damages_nothing() {
        assert_eq!(
            damage(&scene(icons(None)), &scene(icons(None)), 2.0),
            Some(Vec::new())
        );
    }

    #[test]
    fn hovering_an_icon_damages_only_its_highlight() {
        let regions = damage(&scene(icons(None)), &scene(icons(Some(1))), 1.0).unwrap();
        assert_eq!(
            regions,
            vec![Region {
                x: 97,
                y: 9,
                width: 22,
                height: 22
            }]
        );
    }

    #[test]
    fn changes_inside_a_translated_group_land_where_the_group_is_drawn() {
        let moved = |items| Item::Translate {
            offset: Point::new(100.0, 100.0),
            items,
        };
        let regions = damage(
            &scene(vec![fill(0.0, 0x336699), moved(vec![fill(0.0, 0x000000)])]),
            &scene(vec![fill(0.0, 0x336699), moved(vec![fill(0.0, 0xFF0000)])]),
            1.0,
        )
        .unwrap();
        assert_eq!(
            regions,
            vec![Region {
                x: 98,
                y: 108,
                width: 24,
                height: 24
            }]
        );
    }

    #[test]
    fn a_new_size_or_mostly_changed_scene_repaints_everything() {
        let mut taller = scene(icons(None));
        taller.height += 1;
        assert_eq!(damage(&scene(icons(None)), &taller, 1.0), None);
        let cover = |color| {
            vec![Item::Fill {
                shape: Shape::rect(Rect::new(0.0, 0.0, 400.0, 300.0)),
                paint: Color::hex(color).into(),
            }]
        };
        assert_eq!(damage(&scene(cover(0)), &scene(cover(1)), 1.0), None);
    }
}
