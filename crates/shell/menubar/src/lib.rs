use unbound_toolkit::{
    Color, Item, Palette, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem,
};

pub const HEIGHT: u32 = 24;
const INSET: f32 = 16.0;
const TEXT_SIZE: f32 = 13.0;
const ITEM_WIDTH: f32 = 28.0;
const ITEM_SPACING: f32 = 8.0;
const SWITCH_WIDTH: f32 = 14.0;
const SWITCH_HEIGHT: f32 = 6.0;
const SWITCH_GAP: f32 = 3.0;

pub fn clock_text(now: &jiff::Zoned) -> String {
    now.strftime("%a %-d %b %H:%M").to_string()
}

pub fn app_name<'a>(app_id: &'a str, names: &'a [(String, String)]) -> &'a str {
    names
        .iter()
        .find(|(id, _)| id == app_id)
        .map_or(app_id, |(_, name)| name.as_str())
}

pub struct MenuBar {
    palette: Palette,
    text: TextSystem,
    app: String,
    clock: String,
    control_center: Rect,
}

impl MenuBar {
    pub fn new(palette: Palette, text: TextSystem, clock: String) -> Self {
        Self {
            palette,
            text,
            app: String::new(),
            clock,
            control_center: Rect::default(),
        }
    }

    pub fn set_app(&mut self, name: &str) -> bool {
        let changed = self.app != name;
        name.clone_into(&mut self.app);
        changed
    }

    pub fn set_clock(&mut self, clock: String) -> bool {
        let changed = self.clock != clock;
        self.clock = clock;
        changed
    }

    pub fn app(&self) -> &str {
        &self.app
    }

    pub fn control_center_hit(&self, point: Point) -> bool {
        let item = self.control_center;
        point.x >= item.x
            && point.x < item.x + item.width
            && point.y >= item.y
            && point.y < item.y + item.height
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        let bar = Rect::new(0.0, 0.0, width as f32, height as f32);
        let label = self.palette.label_primary;
        let mut items = vec![
            Item::Fill {
                shape: Shape::rect(bar),
                paint: self.palette.surface_elevated.with_alpha(235).into(),
            },
            Item::Fill {
                shape: Shape::rect(Rect::new(0.0, bar.height - 1.0, bar.width, 1.0)),
                paint: self.palette.separator_color.into(),
            },
        ];
        let app = self.text.paragraph(
            &self.app,
            TextStyle::new(TEXT_SIZE, label).weight(TextStyle::SEMIBOLD),
            None,
            TextAlign::Start,
        );
        let clock = self.text.paragraph(
            &self.clock,
            TextStyle::new(TEXT_SIZE, label),
            None,
            TextAlign::Start,
        );
        let middle = |height: f32| ((bar.height - height) / 2.0).round();
        let (app_y, clock_y) = (middle(app.height()), middle(clock.height()));
        let clock_x = (bar.width - INSET - clock.width()).round();
        items.push(Item::Text {
            origin: Point::new(INSET, app_y),
            paragraph: app,
        });
        items.push(Item::Text {
            origin: Point::new(clock_x, clock_y),
            paragraph: clock,
        });
        self.control_center = Rect::new(
            clock_x - ITEM_SPACING - ITEM_WIDTH,
            0.0,
            ITEM_WIDTH,
            bar.height,
        );
        let glyph_x = self.control_center.x + (ITEM_WIDTH - SWITCH_WIDTH) / 2.0;
        let glyph_top = ((bar.height - 2.0 * SWITCH_HEIGHT - SWITCH_GAP) / 2.0).round();
        for (row, knob_at_end) in [(0.0, true), (1.0, false)] {
            let y = glyph_top + row * (SWITCH_HEIGHT + SWITCH_GAP);
            let track = Rect::new(glyph_x, y, SWITCH_WIDTH, SWITCH_HEIGHT);
            items.push(Item::Stroke {
                shape: Shape::rounded(track, SWITCH_HEIGHT / 2.0),
                width: 1.0,
                paint: label.into(),
            });
            let knob_x = if knob_at_end {
                glyph_x + SWITCH_WIDTH - SWITCH_HEIGHT
            } else {
                glyph_x
            };
            items.push(Item::Fill {
                shape: Shape::rounded(
                    Rect::new(knob_x, y, SWITCH_HEIGHT, SWITCH_HEIGHT),
                    SWITCH_HEIGHT / 2.0,
                ),
                paint: label.into(),
            });
        }
        Scene {
            width,
            height,
            background: Color::hex(0).with_alpha(0),
            items,
        }
    }
}
