use std::time::Duration;

use bande_design::{Color, Palette, TextRole};

use crate::{Item, Paragraph, Path, Point, Rect, Shape, TextStyle};

pub const WINDOW_HEADER_HEIGHT: u32 = 48;
pub const FRAME_HEADER_HEIGHT: u32 = 40;
pub const RESIZE_MARGIN: u32 = 8;
pub const RESIZE_INSIDE: f32 = 3.0;
pub const PLACEMENTS_AFTER_REST: Duration = Duration::from_millis(800);
pub const PLACEMENTS_AFTER_HOLD: Duration = Duration::from_millis(500);
pub const HEADER_LEADING: f32 = window_controls_end(true) + TITLE_GAP;
const RESIZE_CORNER_REACH: f32 = 24.0;
const WELL_X: f32 = 12.0;
const WELL_WIDTH: f32 = 66.0;
const WELL_HEIGHT: f32 = 24.0;
const MENU_GAP: f32 = 6.0;
const MENU_REACH: f32 = 4.0;
const MENU_DOT: f32 = 1.25;
const MENU_DOT_PITCH: f32 = 4.5;
const CELL_PITCH: f32 = 21.0;
const FIRST_GLYPH: f32 = 12.0;
const TITLE_GAP: f32 = 16.0;
const TRAILING_INSET: f32 = 12.0;
const GLYPH_STROKE: f32 = 1.5;
const DISC_RADIUS: f32 = 9.0;
const HOVER_OPACITY: f32 = 0.12;
const PRESSED_OPACITY: f32 = 0.22;
const PRESSED_SHADE: f32 = 0.15;
const DISABLED_OPACITY: f32 = 0.45;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowControl {
    Close,
    Minimize,
    Zoom,
    Menu,
}

impl WindowControl {
    pub const ALL: [Self; 3] = [Self::Close, Self::Minimize, Self::Zoom];

    pub fn shown(menu: bool) -> impl Iterator<Item = Self> {
        Self::ALL.into_iter().chain(menu.then_some(Self::Menu))
    }

    pub fn area(self, height: f32) -> Rect {
        let centre = self.centre(height).x;
        let start = match self {
            Self::Close => 0.0,
            Self::Minimize | Self::Zoom => centre - CELL_PITCH / 2.0,
            Self::Menu => WELL_X + WELL_WIDTH,
        };
        let end = match self {
            Self::Zoom => WELL_X + WELL_WIDTH,
            Self::Close | Self::Minimize => centre + CELL_PITCH / 2.0,
            Self::Menu => window_controls_end(true) + MENU_REACH,
        };
        Rect::new(start, 0.0, end - start, height)
    }

    pub fn at(point: Point, height: f32, menu: bool) -> Option<Self> {
        Self::shown(menu).find(|control| control.area(height).contains(point))
    }

    fn centre(self, height: f32) -> Point {
        let x = match self {
            Self::Close => WELL_X + FIRST_GLYPH,
            Self::Minimize => WELL_X + FIRST_GLYPH + CELL_PITCH,
            Self::Zoom => WELL_X + FIRST_GLYPH + 2.0 * CELL_PITCH,
            Self::Menu => WELL_X + WELL_WIDTH + MENU_GAP + WELL_HEIGHT / 2.0,
        };
        Point::new(x, height / 2.0)
    }
}

fn menu_well(height: f32) -> Rect {
    Rect::new(
        WELL_X + WELL_WIDTH + MENU_GAP,
        ((height - WELL_HEIGHT) / 2.0).round(),
        WELL_HEIGHT,
        WELL_HEIGHT,
    )
}

pub fn menu_button_area(height: f32) -> Rect {
    menu_well(height)
}

const fn window_controls_end(menu: bool) -> f32 {
    match menu {
        true => WELL_X + WELL_WIDTH + MENU_GAP + WELL_HEIGHT,
        false => WELL_X + WELL_WIDTH,
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ControlHover {
    pub close: f32,
    pub minimize: f32,
    pub zoom: f32,
    pub menu: f32,
}

impl ControlHover {
    pub fn only(control: Option<WindowControl>) -> Self {
        let level = |this: WindowControl| if control == Some(this) { 1.0 } else { 0.0 };
        Self {
            close: level(WindowControl::Close),
            minimize: level(WindowControl::Minimize),
            zoom: level(WindowControl::Zoom),
            menu: level(WindowControl::Menu),
        }
    }

    fn of(self, control: WindowControl) -> f32 {
        match control {
            WindowControl::Close => self.close,
            WindowControl::Minimize => self.minimize,
            WindowControl::Zoom => self.zoom,
            WindowControl::Menu => self.menu,
        }
    }
}

pub struct TitleBar<'a> {
    pub width: f32,
    pub height: f32,
    pub title: Paragraph,
    pub palette: &'a Palette,
    pub active: bool,
    pub hover: ControlHover,
    pub pressed: Option<WindowControl>,
    pub zoomed: bool,
    pub zoomable: bool,
    pub alternate: bool,
    pub menu_open: Option<bool>,
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
        self.well(well, &mut items);
        for control in WindowControl::ALL {
            self.control(control, &mut items);
        }
        if self.menu_open.is_some() {
            self.well(menu_well(self.height), &mut items);
            self.control(WindowControl::Menu, &mut items);
        }
        let title_x = window_controls_end(self.menu_open.is_some()) + TITLE_GAP;
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

    fn well(&self, well: Rect, items: &mut Vec<Item>) {
        let palette = self.palette;
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
    }

    fn enabled(&self, control: WindowControl) -> bool {
        control != WindowControl::Zoom || self.zoomable
    }

    fn held(&self, control: WindowControl) -> bool {
        self.pressed == Some(control)
            || (control == WindowControl::Menu && self.menu_open == Some(true))
    }

    fn control(&self, control: WindowControl, items: &mut Vec<Item>) {
        let palette = self.palette;
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
        let rest = if self.active {
            palette.ink2
        } else {
            palette.ink3
        };
        let enabled = self.enabled(control);
        let held = enabled && self.held(control);
        let hover = match (enabled, held) {
            (false, _) => 0.0,
            (true, true) => 1.0,
            (true, false) => self.hover.of(control),
        };
        let backfill = match (control, held) {
            (WindowControl::Close, true) => palette.danger.mix(Color::BLACK, PRESSED_SHADE),
            (WindowControl::Close, false) => palette.danger.with_opacity(hover),
            (_, true) => palette.ink.with_opacity(PRESSED_OPACITY),
            (_, false) => palette.ink.with_opacity(HOVER_OPACITY * hover),
        };
        if hover > 0.0 {
            items.push(Item::Fill {
                shape: disc,
                paint: backfill.into(),
            });
        }
        let glyph = match (control, enabled) {
            (WindowControl::Close, _) => rest.mix(palette.on_danger, hover),
            (_, true) => rest.mix(palette.ink, hover),
            (_, false) => rest.with_opacity(DISABLED_OPACITY),
        };
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
            WindowControl::Zoom if self.alternate && self.zoomable => {
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
            WindowControl::Menu => {
                for step in [-1.0, 0.0, 1.0] {
                    let y = centre.y + step * MENU_DOT_PITCH;
                    items.push(Item::Fill {
                        shape: Shape::rounded(
                            Rect::new(
                                centre.x - MENU_DOT,
                                y - MENU_DOT,
                                2.0 * MENU_DOT,
                                2.0 * MENU_DOT,
                            ),
                            MENU_DOT,
                        ),
                        paint: glyph.into(),
                    });
                }
            }
        }
    }
}
