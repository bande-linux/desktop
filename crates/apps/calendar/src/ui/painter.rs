use bande_toolkit::{
    Color, Cut, Item, Paint, Point, Rect, Shape, TextAlign, TextStyle, TextSystem, shortened,
};

pub struct Painter<'a, H> {
    pub text: &'a mut TextSystem,
    pub items: Vec<Item>,
    pub hits: Vec<(Rect, H)>,
}

impl<'a, H> Painter<'a, H> {
    pub fn new(text: &'a mut TextSystem) -> Self {
        Self {
            text,
            items: Vec::new(),
            hits: Vec::new(),
        }
    }

    pub fn fill(&mut self, rect: Rect, radius: f32, color: Color) {
        self.items.push(Item::Fill {
            shape: Shape::rounded(rect, radius),
            paint: Paint::Solid(color),
        });
    }

    pub fn stroke(&mut self, rect: Rect, radius: f32, width: f32, color: Color) {
        self.items.push(Item::Stroke {
            shape: Shape::rounded(rect, radius),
            width,
            paint: Paint::Solid(color),
        });
    }

    pub fn line(&mut self, from: Point, to: Point, width: f32, color: Color) {
        self.items.push(Item::Line {
            from,
            to,
            width,
            paint: Paint::Solid(color),
        });
    }

    pub fn measure(&mut self, text: &str, style: TextStyle) -> f32 {
        self.text
            .paragraph(text, style, None, TextAlign::Start)
            .width()
    }

    pub fn label(&mut self, text: &str, style: TextStyle, frame: Rect, align: TextAlign) {
        let shown = shortened(text, Cut::End, |candidate| {
            self.text.paragraph(candidate, style, None, align).width() <= frame.width + 0.5
        });
        let paragraph = self.text.paragraph(&shown, style, None, align);
        let x = match align {
            TextAlign::Center => frame.x + ((frame.width - paragraph.width()) / 2.0).round(),
            TextAlign::End => frame.x + frame.width - paragraph.width(),
            _ => frame.x,
        };
        let y = frame.y + ((frame.height - paragraph.height()) / 2.0).round();
        self.items.push(Item::Clip {
            shape: Shape::rect(frame),
            items: vec![Item::Text {
                origin: Point::new(x, y),
                paragraph,
            }],
        });
    }

    pub fn wrapped(&mut self, text: &str, style: TextStyle, frame: Rect) -> f32 {
        let line = self
            .text
            .paragraph("X", style, None, TextAlign::Start)
            .height();
        let room = frame.height.max(line) + 0.5;
        let shown = shortened(text, Cut::End, |candidate| {
            let paragraph =
                self.text
                    .paragraph(candidate, style, Some(frame.width), TextAlign::Start);
            paragraph.height() <= room && paragraph.width() <= frame.width + 0.5
        });
        let paragraph = self
            .text
            .paragraph(&shown, style, Some(frame.width), TextAlign::Start);
        let height = paragraph.height();
        self.items.push(Item::Clip {
            shape: Shape::rect(frame),
            items: vec![Item::Text {
                origin: Point::new(frame.x, frame.y),
                paragraph,
            }],
        });
        height
    }

    pub fn hit(&mut self, rect: Rect, hit: H) {
        self.hits.push((rect, hit));
    }

    pub fn clipped(&mut self, shape: Shape, paint: impl FnOnce(&mut Self)) {
        let outer = std::mem::take(&mut self.items);
        paint(self);
        let inner = std::mem::replace(&mut self.items, outer);
        self.items.push(Item::Clip {
            shape,
            items: inner,
        });
    }
}

const CSS_COLORS: &str = "aliceblue:f0f8ff antiquewhite:faebd7 aqua:00ffff aquamarine:7fffd4 azure:f0ffff \
    beige:f5f5dc bisque:ffe4c4 black:000000 blanchedalmond:ffebcd blue:0000ff \
    blueviolet:8a2be2 brown:a52a2a burlywood:deb887 cadetblue:5f9ea0 chartreuse:7fff00 \
    chocolate:d2691e coral:ff7f50 cornflowerblue:6495ed cornsilk:fff8dc crimson:dc143c \
    cyan:00ffff darkblue:00008b darkcyan:008b8b darkgoldenrod:b8860b darkgray:a9a9a9 \
    darkgreen:006400 darkgrey:a9a9a9 darkkhaki:bdb76b darkmagenta:8b008b darkolivegreen:556b2f \
    darkorange:ff8c00 darkorchid:9932cc darkred:8b0000 darksalmon:e9967a darkseagreen:8fbc8f \
    darkslateblue:483d8b darkslategray:2f4f4f darkslategrey:2f4f4f darkturquoise:00ced1 \
    darkviolet:9400d3 deeppink:ff1493 deepskyblue:00bfff dimgray:696969 dimgrey:696969 \
    dodgerblue:1e90ff firebrick:b22222 floralwhite:fffaf0 forestgreen:228b22 fuchsia:ff00ff \
    gainsboro:dcdcdc ghostwhite:f8f8ff gold:ffd700 goldenrod:daa520 gray:808080 green:008000 \
    greenyellow:adff2f grey:808080 honeydew:f0fff0 hotpink:ff69b4 indianred:cd5c5c \
    indigo:4b0082 ivory:fffff0 khaki:f0e68c lavender:e6e6fa lavenderblush:fff0f5 \
    lawngreen:7cfc00 lemonchiffon:fffacd lightblue:add8e6 lightcoral:f08080 lightcyan:e0ffff \
    lightgoldenrodyellow:fafad2 lightgray:d3d3d3 lightgreen:90ee90 lightgrey:d3d3d3 \
    lightpink:ffb6c1 lightsalmon:ffa07a lightseagreen:20b2aa lightskyblue:87cefa \
    lightslategray:778899 lightslategrey:778899 lightsteelblue:b0c4de lightyellow:ffffe0 \
    lime:00ff00 limegreen:32cd32 linen:faf0e6 magenta:ff00ff maroon:800000 \
    mediumaquamarine:66cdaa mediumblue:0000cd mediumorchid:ba55d3 mediumpurple:9370db \
    mediumseagreen:3cb371 mediumslateblue:7b68ee mediumspringgreen:00fa9a \
    mediumturquoise:48d1cc mediumvioletred:c71585 midnightblue:191970 mintcream:f5fffa \
    mistyrose:ffe4e1 moccasin:ffe4b5 navajowhite:ffdead navy:000080 oldlace:fdf5e6 \
    olive:808000 olivedrab:6b8e23 orange:ffa500 orangered:ff4500 orchid:da70d6 \
    palegoldenrod:eee8aa palegreen:98fb98 paleturquoise:afeeee palevioletred:db7093 \
    papayawhip:ffefd5 peachpuff:ffdab9 peru:cd853f pink:ffc0cb plum:dda0dd powderblue:b0e0e6 \
    purple:800080 rebeccapurple:663399 red:ff0000 rosybrown:bc8f8f royalblue:4169e1 \
    saddlebrown:8b4513 salmon:fa8072 sandybrown:f4a460 seagreen:2e8b57 seashell:fff5ee \
    sienna:a0522d silver:c0c0c0 skyblue:87ceeb slateblue:6a5acd slategray:708090 \
    slategrey:708090 snow:fffafa springgreen:00ff7f steelblue:4682b4 tan:d2b48c teal:008080 \
    thistle:d8bfd8 tomato:ff6347 turquoise:40e0d0 violet:ee82ee wheat:f5deb3 white:ffffff \
    whitesmoke:f5f5f5 yellow:ffff00 yellowgreen:9acd32";

pub fn parse_color(text: &str) -> Option<Color> {
    let text = text.trim();
    let hex = CSS_COLORS
        .split_ascii_whitespace()
        .find_map(|entry| {
            let (name, hex) = entry.split_once(':')?;
            name.eq_ignore_ascii_case(text).then_some(hex)
        })
        .unwrap_or_else(|| text.trim_start_matches('#'));
    let channel = |index: usize| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok();
    match hex.len() {
        6 | 8 => Some(Color {
            r: channel(0)?,
            g: channel(2)?,
            b: channel(4)?,
            a: 255,
        }),
        _ => None,
    }
}

pub fn color_text(color: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b)
}
