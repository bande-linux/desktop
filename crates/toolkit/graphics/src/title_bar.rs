use bande_design::{Color, Palette, TextRole};

use crate::{Item, Paragraph, Path, Point, Rect, Shape, TextStyle};

pub const WINDOW_HEADER_HEIGHT: u32 = 48;
pub const FRAME_HEADER_HEIGHT: u32 = 40;
pub const RESIZE_MARGIN: u32 = 8;
pub const RESIZE_INSIDE: f32 = 3.0;
const RESIZE_CORNER_REACH: f32 = 24.0;
const WELL_X: f32 = 12.0;
const WELL_WIDTH: f32 = 66.0;
const WELL_HEIGHT: f32 = 24.0;
const CELL_PITCH: f32 = 21.0;
const FIRST_GLYPH: f32 = 12.0;
const TITLE_GAP: f32 = 16.0;
const TRAILING_INSET: f32 = 12.0;
const GLYPH_STROKE: f32 = 1.5;
const DISC_RADIUS: f32 = 8.0;
const PRESSED_OPACITY: f32 = 0.12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowControl {
    Close,
    Minimize,
    Zoom,
}

impl WindowControl {
    pub const ALL: [Self; 3] = [Self::Close, Self::Minimize, Self::Zoom];

    fn index(self) -> usize {
        match self {
            Self::Close => 0,
            Self::Minimize => 1,
            Self::Zoom => 2,
        }
    }

    pub fn area(self, height: f32) -> Rect {
        let centre = self.centre(height).x;
        let start = match self {
            Self::Close => 0.0,
            Self::Minimize | Self::Zoom => centre - CELL_PITCH / 2.0,
        };
        let end = match self {
            Self::Zoom => WELL_X + WELL_WIDTH,
            Self::Close | Self::Minimize => centre + CELL_PITCH / 2.0,
        };
        Rect::new(start, 0.0, end - start, height)
    }

    pub fn at(point: Point, height: f32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|control| control.area(height).contains(point))
    }

    fn centre(self, height: f32) -> Point {
        Point::new(
            WELL_X + FIRST_GLYPH + self.index() as f32 * CELL_PITCH,
            height / 2.0,
        )
    }
}

pub fn window_controls_area(height: f32) -> Rect {
    Rect::new(0.0, 0.0, WELL_X + WELL_WIDTH, height)
}

pub fn title_style(palette: &Palette, active: bool) -> TextStyle {
    TextStyle::role(
        TextRole::WindowTitle,
        if active { palette.ink } else { palette.ink2 },
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

pub fn resize_zone(window: Rect, radius: f32, outside: f32, point: Point) -> Option<ResizeEdge> {
    if !window.inset(-outside).contains(point)
        || Shape::rounded(window, radius).distance(point) < -RESIZE_INSIDE
    {
        return None;
    }
    let horizontal = side(point.x, window.x, window.width);
    let vertical = side(point.y, window.y, window.height);
    Some(match (vertical, horizontal) {
        (-1, -1) => ResizeEdge::TopLeft,
        (-1, 1) => ResizeEdge::TopRight,
        (1, -1) => ResizeEdge::BottomLeft,
        (1, 1) => ResizeEdge::BottomRight,
        (-1, _) => ResizeEdge::Top,
        (1, _) => ResizeEdge::Bottom,
        (_, -1) => ResizeEdge::Left,
        _ => ResizeEdge::Right,
    })
}

fn side(at: f32, start: f32, length: f32) -> i8 {
    if at < start + RESIZE_CORNER_REACH {
        -1
    } else if at >= start + length - RESIZE_CORNER_REACH {
        1
    } else {
        0
    }
}

pub struct TitleBar<'a> {
    pub width: f32,
    pub height: f32,
    pub title: Paragraph,
    pub palette: &'a Palette,
    pub active: bool,
    pub hover: f32,
    pub close_hover: f32,
    pub pressed: Option<WindowControl>,
    pub zoomed: bool,
    pub alternate: bool,
}

impl TitleBar<'_> {
    pub fn items(&self) -> Vec<Item> {
        let palette = self.palette;
        let mut items = vec![Item::Fill {
            shape: Shape::rect(Rect::new(0.0, 0.0, self.width, self.height)),
            paint: palette.chassis.into(),
        }];
        let well = Rect::new(
            WELL_X,
            ((self.height - WELL_HEIGHT) / 2.0).round(),
            WELL_WIDTH,
            WELL_HEIGHT,
        );
        items.push(if self.active {
            Item::Fill {
                shape: Shape::rounded(well, WELL_HEIGHT / 2.0),
                paint: palette.well.into(),
            }
        } else {
            Item::Stroke {
                shape: Shape::rounded(well.inset(0.5), WELL_HEIGHT / 2.0 - 0.5),
                width: 1.0,
                paint: palette.hairline.into(),
            }
        });
        let rest = if self.active {
            palette.ink2
        } else {
            palette.ink3
        };
        let glyph = rest.mix(palette.ink, self.hover);
        for control in WindowControl::ALL {
            self.control(control, glyph, &mut items);
        }
        let title_x = WELL_X + WELL_WIDTH + TITLE_GAP;
        let available = Rect::new(
            title_x,
            0.0,
            (self.width - title_x - TRAILING_INSET).max(0.0),
            self.height,
        );
        items.push(Item::Clip {
            shape: Shape::rect(available),
            items: vec![Item::Text {
                origin: Point::new(title_x, ((self.height - self.title.height()) / 2.0).round()),
                paragraph: self.title.clone(),
            }],
        });
        items
    }

    fn control(&self, control: WindowControl, glyph: Color, items: &mut Vec<Item>) {
        let centre = control.centre(self.height);
        let disc = Shape::rounded(
            Rect::new(
                centre.x - DISC_RADIUS,
                centre.y - DISC_RADIUS,
                2.0 * DISC_RADIUS,
                2.0 * DISC_RADIUS,
            ),
            DISC_RADIUS,
        );
        let glyph = if control == WindowControl::Close && self.close_hover > 0.0 {
            items.push(Item::Fill {
                shape: disc,
                paint: self.palette.danger.with_opacity(self.close_hover).into(),
            });
            glyph.mix(Color::WHITE, self.close_hover)
        } else {
            glyph
        };
        if self.pressed == Some(control) {
            items.push(Item::Fill {
                shape: disc,
                paint: self.palette.ink.with_opacity(PRESSED_OPACITY).into(),
            });
        }
        let at = |x: f32, y: f32| Point::new(centre.x + x, centre.y + y);
        let stroke = |path: Path| Item::StrokePath {
            path,
            width: GLYPH_STROKE,
            paint: glyph.into(),
        };
        match control {
            WindowControl::Close => {
                items.push(stroke(Path::through(&[at(-3.5, -3.5), at(3.5, 3.5)])));
                items.push(stroke(Path::through(&[at(-3.5, 3.5), at(3.5, -3.5)])));
            }
            WindowControl::Minimize => {
                items.push(stroke(Path::through(&[at(-4.0, 0.0), at(4.0, 0.0)])));
            }
            WindowControl::Zoom if self.alternate => {
                items.push(stroke(Path::through(&[at(-4.0, 0.0), at(4.0, 0.0)])));
                items.push(stroke(Path::through(&[at(0.0, -4.0), at(0.0, 4.0)])));
            }
            WindowControl::Zoom if self.zoomed => {
                items.push(stroke(Path::through(&[
                    at(-4.5, -1.0),
                    at(-1.0, -1.0),
                    at(-1.0, -4.5),
                ])));
                items.push(stroke(Path::through(&[
                    at(4.5, 1.0),
                    at(1.0, 1.0),
                    at(1.0, 4.5),
                ])));
            }
            WindowControl::Zoom => {
                items.push(stroke(Path::through(&[
                    at(-4.5, -1.0),
                    at(-4.5, -4.5),
                    at(-1.0, -4.5),
                ])));
                items.push(stroke(Path::through(&[
                    at(4.5, 1.0),
                    at(4.5, 4.5),
                    at(1.0, 4.5),
                ])));
            }
        }
    }
}
