use std::f32::consts::FRAC_PI_4;

use bande_graphics::{Color, Item, Palette, Path, PathElement, Point, Scheme};
use smithay::input::pointer::CursorIcon;

pub(crate) const CANVAS: u32 = 24;

pub(crate) const SHAPES: [CursorIcon; 36] = [
    CursorIcon::Default,
    CursorIcon::ContextMenu,
    CursorIcon::Help,
    CursorIcon::Pointer,
    CursorIcon::Progress,
    CursorIcon::Wait,
    CursorIcon::Cell,
    CursorIcon::Crosshair,
    CursorIcon::Text,
    CursorIcon::VerticalText,
    CursorIcon::Alias,
    CursorIcon::Copy,
    CursorIcon::Move,
    CursorIcon::NoDrop,
    CursorIcon::NotAllowed,
    CursorIcon::Grab,
    CursorIcon::Grabbing,
    CursorIcon::EResize,
    CursorIcon::NResize,
    CursorIcon::NeResize,
    CursorIcon::NwResize,
    CursorIcon::SResize,
    CursorIcon::SeResize,
    CursorIcon::SwResize,
    CursorIcon::WResize,
    CursorIcon::EwResize,
    CursorIcon::NsResize,
    CursorIcon::NeswResize,
    CursorIcon::NwseResize,
    CursorIcon::ColResize,
    CursorIcon::RowResize,
    CursorIcon::AllScroll,
    CursorIcon::ZoomIn,
    CursorIcon::ZoomOut,
    CursorIcon::DndAsk,
    CursorIcon::AllResize,
];

pub(crate) const LEGACY_NAMES: [(&str, CursorIcon); 6] = [
    ("fleur", CursorIcon::AllScroll),
    ("dnd-copy", CursorIcon::Copy),
    ("dnd-link", CursorIcon::Alias),
    ("dnd-none", CursorIcon::NoDrop),
    ("dnd-move", CursorIcon::Default),
    ("left_ptr_help", CursorIcon::Help),
];

const INK: Color = Color::BLACK;
const PAPER: Color = Color::WHITE;
const RIM: f32 = 2.4;
const SHADOW_OPACITY: f32 = 0.28;
const SHADOW_DROP: f32 = 0.7;
const CENTRE: Point = Point::new(12.0, 12.0);
const BADGE: Point = Point::new(17.6, 17.4);
const BADGE_RADIUS: f32 = 4.8;
const ARROW_TIP: Point = Point::new(5.0, 3.0);
const ARROW: [(f32, f32); 7] = [
    (5.0, 3.0),
    (5.0, 19.0),
    (8.8, 15.4),
    (11.4, 21.0),
    (14.0, 19.8),
    (11.5, 14.4),
    (16.7, 14.4),
];
const STRAIGHT_REACH: f32 = 9.6;
const DIAGONAL_REACH: f32 = 8.6;
const KAPPA: f32 = 0.552_284_8;

pub(crate) struct Drawing {
    pub(crate) items: Vec<Item>,
    pub(crate) hotspot: Point,
}

enum Part {
    Area(Path),
    Line(Path, f32),
}

struct Glyph {
    parts: Vec<Part>,
    ink: Color,
    rim: Color,
}

enum Badge {
    Menu,
    Question,
    Spinner,
    Plus,
    Link,
    Forbidden,
}

enum Hand {
    Pointing,
    Open,
    Closed,
}

pub(crate) fn drawing(icon: CursorIcon) -> Drawing {
    match icon {
        CursorIcon::ContextMenu | CursorIcon::DndAsk => arrow(Some(Badge::Menu)),
        CursorIcon::Help => arrow(Some(Badge::Question)),
        CursorIcon::Progress => arrow(Some(Badge::Spinner)),
        CursorIcon::Copy => arrow(Some(Badge::Plus)),
        CursorIcon::Alias => arrow(Some(Badge::Link)),
        CursorIcon::NoDrop | CursorIcon::NotAllowed => arrow(Some(Badge::Forbidden)),
        CursorIcon::Wait => centred(spinner(CENTRE, 9.0)),
        CursorIcon::Pointer => hand(Hand::Pointing),
        CursorIcon::Grab => hand(Hand::Open),
        CursorIcon::Grabbing => hand(Hand::Closed),
        CursorIcon::Text => centred(beam(false)),
        CursorIcon::VerticalText => centred(beam(true)),
        CursorIcon::Crosshair => centred(crosshair()),
        CursorIcon::Cell => centred(cell()),
        CursorIcon::EResize | CursorIcon::WResize | CursorIcon::EwResize => {
            centred(double_arrow(0.0, STRAIGHT_REACH))
        }
        CursorIcon::NResize | CursorIcon::SResize | CursorIcon::NsResize => {
            centred(double_arrow(2.0 * FRAC_PI_4, STRAIGHT_REACH))
        }
        CursorIcon::NeResize | CursorIcon::SwResize | CursorIcon::NeswResize => {
            centred(double_arrow(-FRAC_PI_4, DIAGONAL_REACH))
        }
        CursorIcon::NwResize | CursorIcon::SeResize | CursorIcon::NwseResize => {
            centred(double_arrow(FRAC_PI_4, DIAGONAL_REACH))
        }
        CursorIcon::ColResize => centred(split(0.0)),
        CursorIcon::RowResize => centred(split(2.0 * FRAC_PI_4)),
        CursorIcon::Move | CursorIcon::AllScroll | CursorIcon::AllResize => centred(four_way()),
        CursorIcon::ZoomIn => magnifier(true),
        CursorIcon::ZoomOut => magnifier(false),
        _ => arrow(None),
    }
}

fn centred(items: Vec<Item>) -> Drawing {
    Drawing {
        items,
        hotspot: CENTRE,
    }
}

fn point(x: f32, y: f32) -> Point {
    Point::new(x, y)
}

fn polygon(points: impl IntoIterator<Item = Point>) -> Path {
    let points: Vec<Point> = points.into_iter().collect();
    Path::through(&points).close()
}

fn segment(from: Point, to: Point) -> Path {
    Path::new().move_to(from).line_to(to)
}

fn circle(centre: Point, radius: f32) -> Path {
    let k = radius * KAPPA;
    let (x, y) = (centre.x, centre.y);
    Path::new()
        .move_to(point(x + radius, y))
        .cubic_to(
            point(x + radius, y + k),
            point(x + k, y + radius),
            point(x, y + radius),
        )
        .cubic_to(
            point(x - k, y + radius),
            point(x - radius, y + k),
            point(x - radius, y),
        )
        .cubic_to(
            point(x - radius, y - k),
            point(x - k, y - radius),
            point(x, y - radius),
        )
        .cubic_to(
            point(x + k, y - radius),
            point(x + radius, y - k),
            point(x + radius, y),
        )
        .close()
}

fn rounded(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Path {
    let r = radius.min(width / 2.0).min(height / 2.0);
    let k = r * (1.0 - KAPPA);
    let (right, bottom) = (x + width, y + height);
    Path::new()
        .move_to(point(x + r, y))
        .line_to(point(right - r, y))
        .cubic_to(
            point(right - k, y),
            point(right, y + k),
            point(right, y + r),
        )
        .line_to(point(right, bottom - r))
        .cubic_to(
            point(right, bottom - k),
            point(right - k, bottom),
            point(right - r, bottom),
        )
        .line_to(point(x + r, bottom))
        .cubic_to(
            point(x + k, bottom),
            point(x, bottom - k),
            point(x, bottom - r),
        )
        .line_to(point(x, y + r))
        .cubic_to(point(x, y + k), point(x + k, y), point(x + r, y))
        .close()
}

fn turned(path: &Path, about: Point, radians: f32) -> Path {
    let (sin, cos) = radians.sin_cos();
    let turn = |p: Point| {
        let (dx, dy) = (p.x - about.x, p.y - about.y);
        point(about.x + dx * cos - dy * sin, about.y + dx * sin + dy * cos)
    };
    path.elements()
        .iter()
        .fold(Path::new(), |turned, element| match *element {
            PathElement::Move(p) => turned.move_to(turn(p)),
            PathElement::Line(p) => turned.line_to(turn(p)),
            PathElement::Cubic(a, b, c) => turned.cubic_to(turn(a), turn(b), turn(c)),
            PathElement::Close => turned.close(),
        })
}

fn along(radians: f32, local: &[(f32, f32)]) -> Path {
    let (sin, cos) = radians.sin_cos();
    polygon(
        local
            .iter()
            .map(|(x, y)| point(CENTRE.x + x * cos - y * sin, CENTRE.y + x * sin + y * cos)),
    )
}

fn draw(glyphs: &[Glyph]) -> Vec<Item> {
    let silhouettes = glyphs
        .iter()
        .flat_map(|glyph| &glyph.parts)
        .map(|part| match part {
            Part::Area(path) => Item::StrokePath {
                path: path.clone(),
                width: RIM + 0.8,
                paint: INK.into(),
            },
            Part::Line(path, width) => Item::StrokePath {
                path: path.clone(),
                width: width + RIM + 0.8,
                paint: INK.into(),
            },
        })
        .collect();
    let mut items = vec![Item::Opacity {
        alpha: SHADOW_OPACITY,
        items: vec![Item::Translate {
            offset: point(0.0, SHADOW_DROP),
            items: silhouettes,
        }],
    }];
    for glyph in glyphs {
        items.extend(glyph.parts.iter().map(|part| match part {
            Part::Area(path) => Item::StrokePath {
                path: path.clone(),
                width: RIM,
                paint: glyph.rim.into(),
            },
            Part::Line(path, width) => Item::StrokePath {
                path: path.clone(),
                width: width + RIM,
                paint: glyph.rim.into(),
            },
        }));
        items.extend(glyph.parts.iter().map(|part| match part {
            Part::Area(path) => Item::FillPath {
                path: path.clone(),
                paint: glyph.ink.into(),
            },
            Part::Line(path, width) => Item::StrokePath {
                path: path.clone(),
                width: *width,
                paint: glyph.ink.into(),
            },
        }));
    }
    items
}

fn dark(parts: Vec<Part>) -> Glyph {
    Glyph {
        parts,
        ink: INK,
        rim: PAPER,
    }
}

fn light(parts: Vec<Part>) -> Glyph {
    Glyph {
        parts,
        ink: PAPER,
        rim: INK,
    }
}

fn arrow(badge: Option<Badge>) -> Drawing {
    let body = dark(vec![Part::Area(polygon(
        ARROW.iter().map(|(x, y)| point(*x, *y)),
    ))]);
    let mut items = draw(&[body]);
    if let Some(badge) = badge {
        items.extend(badged(badge));
    }
    Drawing {
        items,
        hotspot: ARROW_TIP,
    }
}

fn badged(badge: Badge) -> Vec<Item> {
    let palette = Palette::for_scheme(Scheme::Light);
    let disc = |ink: Color| Glyph {
        parts: vec![Part::Area(circle(BADGE, BADGE_RADIUS))],
        ink,
        rim: PAPER,
    };
    let (x, y) = (BADGE.x, BADGE.y);
    let mark = |path: Path, width: f32, paint: Color| Item::StrokePath {
        path,
        width,
        paint: paint.into(),
    };
    match badge {
        Badge::Plus => {
            let mut items = draw(&[disc(palette.success)]);
            items.push(mark(
                segment(point(x - 2.4, y), point(x + 2.4, y)),
                1.7,
                PAPER,
            ));
            items.push(mark(
                segment(point(x, y - 2.4), point(x, y + 2.4)),
                1.7,
                PAPER,
            ));
            items
        }
        Badge::Forbidden => {
            let mut items = draw(&[disc(palette.danger)]);
            items.push(mark(
                segment(point(x - 2.5, y), point(x + 2.5, y)),
                1.9,
                PAPER,
            ));
            items
        }
        Badge::Question => {
            let mut items = draw(&[disc(INK)]);
            let hook = Path::new()
                .move_to(point(x - 1.7, y - 1.2))
                .cubic_to(
                    point(x - 1.7, y - 3.3),
                    point(x + 1.8, y - 3.3),
                    point(x + 1.8, y - 1.2),
                )
                .cubic_to(point(x + 1.8, y), point(x, y + 0.1), point(x, y + 1.2));
            items.push(mark(hook, 1.4, PAPER));
            items.push(Item::FillPath {
                path: circle(point(x, y + 2.9), 0.85),
                paint: PAPER.into(),
            });
            items
        }
        Badge::Link => {
            let mut items = draw(&[disc(INK)]);
            let curve = Path::new().move_to(point(x - 2.2, y + 2.4)).cubic_to(
                point(x - 2.2, y - 0.8),
                point(x - 0.6, y - 1.7),
                point(x + 2.0, y - 1.7),
            );
            let head = Path::through(&[
                point(x + 0.2, y - 3.5),
                point(x + 2.0, y - 1.7),
                point(x + 0.2, y + 0.1),
            ]);
            items.push(mark(curve, 1.4, PAPER));
            items.push(mark(head, 1.4, PAPER));
            items
        }
        Badge::Menu => {
            let card = light(vec![Part::Area(rounded(x - 4.4, y - 3.8, 8.8, 7.6, 1.6))]);
            let mut items = draw(&[card]);
            items.extend((0..3).map(|row| {
                let at = y - 1.9 + row as f32 * 1.9;
                mark(segment(point(x - 2.4, at), point(x + 2.4, at)), 1.0, INK)
            }));
            items
        }
        Badge::Spinner => spinner(BADGE, BADGE_RADIUS),
    }
}

fn spinner(centre: Point, radius: f32) -> Vec<Item> {
    let mut items = draw(&[dark(vec![Part::Area(circle(centre, radius))])]);
    let spokes = 8;
    items.extend((0..spokes).map(|spoke| {
        let angle = spoke as f32 * 2.0 * FRAC_PI_4;
        let (sin, cos) = angle.sin_cos();
        let at = |reach: f32| point(centre.x + cos * reach, centre.y + sin * reach);
        Item::StrokePath {
            path: segment(at(radius * 0.38), at(radius * 0.74)),
            width: radius * 0.2,
            paint: PAPER
                .with_opacity((spoke + 1) as f32 / spokes as f32)
                .into(),
        }
    }));
    items
}

fn beam(vertical: bool) -> Vec<Item> {
    let path = Path::new()
        .move_to(point(9.4, 3.6))
        .line_to(point(10.9, 3.6))
        .line_to(point(12.0, 4.7))
        .line_to(point(13.1, 3.6))
        .line_to(point(14.6, 3.6))
        .move_to(point(12.0, 4.7))
        .line_to(point(12.0, 19.3))
        .move_to(point(9.4, 20.4))
        .line_to(point(10.9, 20.4))
        .line_to(point(12.0, 19.3))
        .line_to(point(13.1, 20.4))
        .line_to(point(14.6, 20.4));
    let path = if vertical {
        turned(&path, CENTRE, 2.0 * FRAC_PI_4)
    } else {
        path
    };
    draw(&[dark(vec![Part::Line(path, 1.4)])])
}

fn crosshair() -> Vec<Item> {
    let arms = [
        segment(point(12.0, 2.5), point(12.0, 9.5)),
        segment(point(12.0, 14.5), point(12.0, 21.5)),
        segment(point(2.5, 12.0), point(9.5, 12.0)),
        segment(point(14.5, 12.0), point(21.5, 12.0)),
    ];
    draw(&[dark(
        arms.into_iter().map(|arm| Part::Line(arm, 1.4)).collect(),
    )])
}

fn cell() -> Vec<Item> {
    let (arm, reach) = (2.1, 7.6);
    let plus = along(
        0.0,
        &[
            (-arm, -reach),
            (arm, -reach),
            (arm, -arm),
            (reach, -arm),
            (reach, arm),
            (arm, arm),
            (arm, reach),
            (-arm, reach),
            (-arm, arm),
            (-reach, arm),
            (-reach, -arm),
            (-arm, -arm),
        ],
    );
    draw(&[light(vec![Part::Area(plus)])])
}

fn two_headed(radians: f32, reach: f32) -> Path {
    let (head, wing, shaft) = (4.6, 4.4, 1.2);
    along(
        radians,
        &[
            (-reach, 0.0),
            (-reach + head, -wing),
            (-reach + head, -shaft),
            (reach - head, -shaft),
            (reach - head, -wing),
            (reach, 0.0),
            (reach - head, wing),
            (reach - head, shaft),
            (-reach + head, shaft),
            (-reach + head, wing),
        ],
    )
}

fn double_arrow(radians: f32, reach: f32) -> Vec<Item> {
    draw(&[dark(vec![Part::Area(two_headed(radians, reach))])])
}

fn four_way() -> Vec<Item> {
    draw(&[dark(vec![
        Part::Area(two_headed(0.0, 9.8)),
        Part::Area(two_headed(2.0 * FRAC_PI_4, 9.8)),
    ])])
}

fn split(radians: f32) -> Vec<Item> {
    let (reach, head, wing, shaft, gap) = (10.0, 4.4, 4.2, 1.1, 2.4);
    let half = |side: f32| {
        along(
            radians,
            &[
                (side * reach, 0.0),
                (side * (reach - head), -wing),
                (side * (reach - head), -shaft),
                (side * gap, -shaft),
                (side * gap, shaft),
                (side * (reach - head), shaft),
                (side * (reach - head), wing),
            ],
        )
    };
    let bar = along(
        radians,
        &[(-1.0, -8.0), (1.0, -8.0), (1.0, 8.0), (-1.0, 8.0)],
    );
    draw(&[dark(vec![
        Part::Area(half(-1.0)),
        Part::Area(half(1.0)),
        Part::Area(bar),
    ])])
}

fn magnifier(enlarge: bool) -> Drawing {
    let lens = point(10.0, 10.0);
    let handle = dark(vec![Part::Line(
        segment(point(14.6, 14.6), point(20.0, 20.0)),
        3.0,
    )]);
    let glass = Glyph {
        parts: vec![Part::Area(circle(lens, 6.0))],
        ink: PAPER,
        rim: INK,
    };
    let mut items = draw(&[handle]);
    items.push(Item::StrokePath {
        path: circle(lens, 6.0),
        width: RIM + 2.4,
        paint: PAPER.into(),
    });
    items.extend(draw(&[glass]));
    let line = |from: Point, to: Point| Item::StrokePath {
        path: segment(from, to),
        width: 1.5,
        paint: INK.into(),
    };
    items.push(line(point(7.4, 10.0), point(12.6, 10.0)));
    if enlarge {
        items.push(line(point(10.0, 7.4), point(10.0, 12.6)));
    }
    Drawing {
        items,
        hotspot: lens,
    }
}

fn hand(pose: Hand) -> Drawing {
    let finger = |x: f32, top: f32, width: f32, bottom: f32| {
        Part::Area(rounded(x, top, width, bottom - top, width / 2.0))
    };
    let thumb = |x: f32, y: f32, length: f32, radians: f32| {
        Part::Area(turned(
            &rounded(x, y, length, 2.9, 1.45),
            point(x + length, y + 1.45),
            radians,
        ))
    };
    let (parts, gaps, hotspot) = match pose {
        Hand::Pointing => (
            vec![
                thumb(3.2, 12.6, 5.6, 0.7),
                finger(8.0, 1.6, 3.0, 13.0),
                finger(11.0, 8.0, 3.0, 14.0),
                finger(14.0, 9.0, 3.0, 14.5),
                finger(17.0, 10.4, 2.7, 15.0),
                Part::Area(rounded(7.0, 11.4, 12.7, 10.2, 3.6)),
            ],
            vec![(11.0, 8.9, 12.6), (14.0, 9.9, 12.9), (17.0, 11.3, 13.2)],
            point(9.5, 2.2),
        ),
        Hand::Open => (
            vec![
                thumb(2.6, 11.2, 5.8, 0.6),
                finger(6.2, 5.0, 2.8, 14.0),
                finger(9.2, 3.0, 2.8, 13.0),
                finger(12.2, 3.6, 2.8, 13.0),
                finger(15.2, 5.6, 2.6, 13.6),
                Part::Area(rounded(6.2, 10.0, 11.6, 10.6, 3.8)),
            ],
            vec![(9.2, 5.6, 11.4), (12.2, 5.6, 11.4), (15.2, 6.8, 11.6)],
            CENTRE,
        ),
        Hand::Closed => (
            vec![
                thumb(4.2, 11.6, 4.8, 0.25),
                finger(6.2, 8.0, 2.8, 13.0),
                finger(9.2, 7.2, 2.8, 13.0),
                finger(12.2, 7.4, 2.8, 13.0),
                finger(15.2, 8.0, 2.6, 13.0),
                Part::Area(rounded(6.2, 10.0, 11.6, 10.0, 3.8)),
            ],
            vec![(9.2, 8.6, 11.0), (12.2, 8.6, 11.0), (15.2, 8.8, 11.0)],
            CENTRE,
        ),
    };
    let mut items = draw(&[light(parts)]);
    items.extend(gaps.into_iter().map(|(x, top, bottom)| Item::StrokePath {
        path: segment(point(x, top), point(x, bottom)),
        width: 0.9,
        paint: INK.into(),
    }));
    Drawing { items, hotspot }
}
