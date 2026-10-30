use crate::{Color, Item, Palette, Paragraph, Point, Rect, Scheme, Shape, TextStyle};

pub const TITLE_BAR_HEIGHT: u32 = 28;
pub const RESIZE_MARGIN: u32 = 6;
pub const RESIZE_CORNER: f32 = 14.0;
const CONTROL_DIAMETER: f32 = 12.0;
const CONTROL_PITCH: f32 = 20.0;
const CONTROLS_INSET: f32 = 12.0;
const TITLE_SIZE: f32 = 13.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowControl {
    Close,
    Minimize,
    Zoom,
}

impl WindowControl {
    pub const ALL: [Self; 3] = [Self::Close, Self::Minimize, Self::Zoom];

    pub fn area(self, height: f32) -> Rect {
        let centre = self.centre(height);
        Rect::new(centre.x - CONTROL_PITCH / 2.0, 0.0, CONTROL_PITCH, height)
    }

    pub fn at(point: Point, height: f32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|control| contains(control.area(height), point))
    }

    fn centre(self, height: f32) -> Point {
        let index = Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .unwrap_or(0);
        Point::new(
            CONTROLS_INSET + CONTROL_DIAMETER / 2.0 + index as f32 * CONTROL_PITCH,
            height / 2.0,
        )
    }

    fn colors(self) -> (Color, Color, Color) {
        match self {
            Self::Close => (
                Color::hex(0xFF5F57),
                Color::hex(0xE24B41),
                Color::hex(0x7A0F08),
            ),
            Self::Minimize => (
                Color::hex(0xFEBC2E),
                Color::hex(0xE1A116),
                Color::hex(0x8A5300),
            ),
            Self::Zoom => (
                Color::hex(0x28C840),
                Color::hex(0x1AAB29),
                Color::hex(0x0B5E14),
            ),
        }
    }
}

pub fn window_controls_area(height: f32) -> Rect {
    let first = WindowControl::Close.area(height);
    Rect::new(first.x, first.y, CONTROL_PITCH * 3.0, height)
}

pub fn title_style(palette: &Palette, active: bool) -> TextStyle {
    let color = if active {
        palette.label_primary
    } else {
        palette.label_muted
    };
    TextStyle::new(TITLE_SIZE, color).weight(TextStyle::SEMIBOLD)
}

fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x
        && point.y >= rect.y
        && point.x < rect.x + rect.width
        && point.y < rect.y + rect.height
}

pub struct TitleBar<'a> {
    pub width: f32,
    pub height: f32,
    pub title: Paragraph,
    pub palette: &'a Palette,
    pub scheme: Scheme,
    pub active: bool,
    pub glyphs: f32,
    pub pressed: Option<WindowControl>,
}

impl TitleBar<'_> {
    pub fn items(&self) -> Vec<Item> {
        let mut items = vec![
            Item::Fill {
                shape: Shape::rect(Rect::new(0.0, 0.0, self.width, self.height)),
                paint: self.palette.surface_elevated.into(),
            },
            Item::Fill {
                shape: Shape::rect(Rect::new(0.0, self.height - 1.0, self.width, 1.0)),
                paint: self.palette.separator_color.into(),
            },
        ];
        let reserved = window_controls_area(self.height);
        let side = reserved.x + reserved.width + 8.0;
        let available = Rect::new(side, 0.0, (self.width - 2.0 * side).max(0.0), self.height);
        let x = ((self.width - self.title.width()) / 2.0)
            .max(available.x)
            .round();
        let y = ((self.height - self.title.height()) / 2.0).round();
        items.push(Item::Clip {
            shape: Shape::rect(available),
            items: vec![Item::Text {
                origin: Point::new(x, y),
                paragraph: self.title.clone(),
            }],
        });
        let lit = if self.active { 1.0 } else { self.glyphs };
        for control in WindowControl::ALL {
            self.control(control, lit, &mut items);
        }
        items
    }

    fn control(&self, control: WindowControl, lit: f32, items: &mut Vec<Item>) {
        let centre = control.centre(self.height);
        let half = CONTROL_DIAMETER / 2.0;
        let disc = Rect::new(
            centre.x - half,
            centre.y - half,
            CONTROL_DIAMETER,
            CONTROL_DIAMETER,
        );
        let (dim_fill, dim_edge) = unlit_colors(self.scheme);
        let (lit_fill, lit_edge, glyph) = control.colors();
        let (fill, edge) = (dim_fill.mix(lit_fill, lit), dim_edge.mix(lit_edge, lit));
        let fill = if self.pressed == Some(control) {
            darker(fill)
        } else {
            fill
        };
        items.push(Item::Fill {
            shape: Shape::rounded(disc, half),
            paint: fill.into(),
        });
        items.push(Item::Stroke {
            shape: Shape::rounded(disc.inset(0.25), half - 0.25),
            width: 0.5,
            paint: edge.into(),
        });
        if self.glyphs <= 0.0 {
            return;
        }
        let line = |from: (f32, f32), to: (f32, f32)| Item::Line {
            from: Point::new(centre.x + from.0, centre.y + from.1),
            to: Point::new(centre.x + to.0, centre.y + to.1),
            width: 1.1,
            paint: glyph.with_alpha((200.0 * self.glyphs).round() as u8).into(),
        };
        let arm = 2.75;
        match control {
            WindowControl::Close => {
                items.push(line((-arm, -arm), (arm, arm)));
                items.push(line((-arm, arm), (arm, -arm)));
            }
            WindowControl::Minimize => items.push(line((-3.5, 0.0), (3.5, 0.0))),
            WindowControl::Zoom => {
                items.push(line((-3.5, 0.0), (3.5, 0.0)));
                items.push(line((0.0, -3.5), (0.0, 3.5)));
            }
        }
    }
}

fn unlit_colors(scheme: Scheme) -> (Color, Color) {
    match scheme {
        Scheme::Light => (Color::hex(0xD9DADD), Color::hex(0xC3C5CA)),
        Scheme::Dark => (Color::hex(0x4A4D54), Color::hex(0x5A5E66)),
    }
}

fn darker(color: Color) -> Color {
    let scale = |channel: u8| (u16::from(channel) * 4 / 5) as u8;
    Color {
        r: scale(color.r),
        g: scale(color.g),
        b: scale(color.b),
        a: color.a,
    }
}
