use crate::Color;

pub fn wcag(a: Color, b: Color) -> f64 {
    let luminance = |color: Color| {
        let [r, g, b] = color.linear();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

pub fn apca(text: Color, background: Color) -> f64 {
    let luminance = |color: Color| {
        let [r, g, b] =
            [color.r, color.g, color.b].map(|channel| (f64::from(channel) / 255.0).powf(2.4));
        let y = 0.212_672_9 * r + 0.715_152_2 * g + 0.072_175 * b;
        if y > 0.022 {
            y
        } else {
            y + (0.022 - y).powf(1.414)
        }
    };
    let (text, background) = (luminance(text), luminance(background));
    if background > text {
        let contrast = (background.powf(0.56) - text.powf(0.57)) * 1.14;
        if contrast < 0.1 {
            0.0
        } else {
            (contrast - 0.027) * 100.0
        }
    } else {
        let contrast = (background.powf(0.65) - text.powf(0.62)) * 1.14;
        if contrast > -0.1 {
            0.0
        } else {
            (contrast + 0.027) * 100.0
        }
    }
}
