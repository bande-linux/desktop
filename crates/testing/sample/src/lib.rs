use unbound_toolkit::{
    Color, Item, Palette, Point, Rect, Scene, Shape, Spacing, TextAlign, TextStyle, TextSystem,
};

pub const SIZE: (u32, u32) = (640, 400);

const ROWS: [(&str, &str); 4] = [
    ("Displays", "Built-in Retina Display, 120 Hz"),
    ("Sound", "MacBook Pro Speakers"),
    ("Network", "Wi-Fi connected to Unbound"),
    ("Keyboard", "Key repeat fast, delay short"),
];

pub fn scene(text: &mut TextSystem, palette: &Palette, (width, height): (u32, u32)) -> Scene {
    let inset = Spacing::XL as f32;
    let card = Rect::new(
        inset,
        inset,
        width as f32 - 2.0 * inset,
        height as f32 - 2.0 * inset,
    );
    let content = card.x + Spacing::L as f32;
    let mut items = vec![
        Item::Shadow {
            shape: Shape::rounded(card, 10.0),
            offset: Point::new(0.0, 2.0),
            blur_radius: 8.0,
            color: Color::hex(0x000000).with_alpha(40),
        },
        Item::Fill {
            shape: Shape::rounded(card, 10.0),
            paint: palette.overlay.into(),
        },
        Item::Text {
            origin: Point::new(content, card.y + Spacing::L as f32),
            paragraph: text.paragraph(
                "Settings",
                TextStyle::new(22.0, palette.ink).weight(TextStyle::SEMIBOLD),
                None,
                TextAlign::Start,
            ),
        },
    ];
    let mut y = card.y + 64.0;
    for (title, detail) in ROWS {
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(
                content,
                y,
                card.width - 2.0 * Spacing::L as f32,
                1.0,
            )),
            paint: palette.hairline.into(),
        });
        items.push(Item::Text {
            origin: Point::new(content, y + Spacing::M as f32),
            paragraph: text.paragraph(
                title,
                TextStyle::new(13.0, palette.ink).weight(TextStyle::MEDIUM),
                None,
                TextAlign::Start,
            ),
        });
        items.push(Item::Text {
            origin: Point::new(content + 160.0, y + Spacing::M as f32),
            paragraph: text.paragraph(
                detail,
                TextStyle::new(13.0, palette.ink2),
                Some(card.width - 192.0),
                TextAlign::Start,
            ),
        });
        y += 48.0;
    }
    Scene {
        width,
        height,
        background: palette.sheet,
        items,
    }
}
