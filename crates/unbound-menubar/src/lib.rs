use unbound_toolkit::{
    Color, Item, Palette, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem,
};

pub const HEIGHT: u32 = 24;
const INSET: f32 = 16.0;
const TEXT_SIZE: f32 = 13.0;

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
}

impl MenuBar {
    pub fn new(palette: Palette, text: TextSystem, clock: String) -> Self {
        Self {
            palette,
            text,
            app: String::new(),
            clock,
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
        Scene {
            width,
            height,
            background: Color::hex(0).with_alpha(0),
            items,
        }
    }
}
