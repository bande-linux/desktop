use std::collections::HashMap;

use bande_status::status::{Battery, Wifi, wifi_bars};
use bande_toolkit::{Color, Image, Item, Palette, Rect, Shape};

const BATTERY_WIDTH: f32 = 23.0;
const BATTERY_HEIGHT: f32 = 11.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Symbol {
    Logo,
    Wifi { bars: u8 },
    WifiOff,
    Bolt,
}

const WIFI_ARCS: [&str; 3] = [
    "M6.17 10.17A4 4 0 0 1 11.83 10.17",
    "M3.34 7.34A8 8 0 0 1 14.66 7.34",
    "M0.9 4.6A11.6 11.6 0 0 1 17.1 4.6",
];

fn hex(color: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

fn arcs(paths: &[&str], lit: u8, color: &str, width: f32) -> String {
    paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let opacity = if index < usize::from(lit) { 1.0 } else { 0.3 };
            format!(
                r#"<path d="{path}" fill="none" stroke="{color}" stroke-opacity="{opacity}" stroke-width="{width}" stroke-linecap="round"/>"#
            )
        })
        .collect()
}

pub fn svg(symbol: Symbol, color: Color) -> String {
    let color = hex(color);
    let (view, body) = match symbol {
        Symbol::Logo => (
            "0 0 16 16",
            format!(
                r#"<mask id="b"><rect width="16" height="16" fill="white"/><path d="M5.33 3.2V12.8M5.33 3.2H7.47A2.13 2.13 0 0 1 7.47 7.47H5.33M5.33 7.47H8A2.67 2.67 0 0 1 8 12.8H5.33" fill="none" stroke="black" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"/></mask><rect x="1" y="1" width="14" height="14" rx="4.2" fill="{color}" mask="url(#b)"/>"#
            ),
        ),
        Symbol::Wifi { bars } => (
            "0 0 18 14",
            format!(
                r#"{}<circle cx="9" cy="12.4" r="1.5" fill="{color}"/>"#,
                arcs(&WIFI_ARCS, bars, &color, 1.8)
            ),
        ),
        Symbol::WifiOff => (
            "0 0 18 14",
            format!(
                r#"{}<circle cx="9" cy="12.4" r="1.5" fill="{color}" fill-opacity="0.3"/><path d="M3 1.5L15 13.5" stroke="{color}" stroke-width="1.6" stroke-linecap="round"/>"#,
                arcs(&WIFI_ARCS, 0, &color, 1.8)
            ),
        ),
        Symbol::Bolt => (
            "0 0 8 12",
            format!(r#"<path d="M5 0L0.4 6.9H3.6L2.6 12L7.6 4.8H4.4z" fill="{color}"/>"#),
        ),
    };
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view}">{body}</svg>"#)
}

pub fn natural_size(symbol: Symbol) -> (f32, f32) {
    match symbol {
        Symbol::Logo => (15.0, 15.0),
        Symbol::Wifi { .. } | Symbol::WifiOff => (17.0, 13.0),
        Symbol::Bolt => (6.0, 9.0),
    }
}

#[derive(Default)]
pub struct Symbols {
    cache: HashMap<(Symbol, Color, u32, u32), Option<Image>>,
}

impl Symbols {
    pub fn image(&mut self, symbol: Symbol, color: Color, scale: f64) -> Option<Image> {
        self.image_at(symbol, color, natural_size(symbol), scale)
    }

    pub fn image_at(
        &mut self,
        symbol: Symbol,
        color: Color,
        (width, height): (f32, f32),
        scale: f64,
    ) -> Option<Image> {
        let pixels = |points: f32| ((f64::from(points) * scale).round() as u32).max(1);
        let key = (symbol, color, pixels(width), pixels(height));
        self.cache
            .entry(key)
            .or_insert_with(|| Image::from_svg(svg(symbol, color).as_bytes(), key.2, key.3))
            .clone()
    }
}

pub fn snap(value: f32, scale: f64) -> f32 {
    ((f64::from(value) * scale).round() / scale) as f32
}

pub fn wifi_symbol(wifi: Option<&Wifi>) -> Symbol {
    match wifi {
        Some(wifi) if wifi.enabled => Symbol::Wifi {
            bars: wifi
                .current()
                .map_or(0, |network| wifi_bars(network.strength)),
        },
        _ => Symbol::WifiOff,
    }
}

pub fn place_symbol(
    symbols: &mut Symbols,
    symbol: Symbol,
    color: Color,
    rect: Rect,
    scale: f64,
    items: &mut Vec<Item>,
) {
    let (width, height) = natural_size(symbol);
    let Some(image) = symbols.image(symbol, color, scale) else {
        return;
    };
    items.push(Item::Image {
        rect: Rect::new(
            snap(rect.x + (rect.width - width) / 2.0, scale),
            snap(rect.y + (rect.height - height) / 2.0, scale),
            image.width() as f32 / scale as f32,
            image.height() as f32 / scale as f32,
        ),
        image,
    });
}

pub fn battery_items(
    symbols: &mut Symbols,
    palette: &Palette,
    battery: Battery,
    rect: Rect,
    scale: f64,
    items: &mut Vec<Item>,
) {
    let label = palette.ink;
    let x = snap(rect.x + (rect.width - BATTERY_WIDTH - 2.0) / 2.0, scale);
    let y = snap(rect.y + (rect.height - BATTERY_HEIGHT) / 2.0, scale);
    let body = Rect::new(x, y, BATTERY_WIDTH, BATTERY_HEIGHT);
    items.push(Item::Stroke {
        shape: Shape::rounded(body.inset(0.5), 3.0),
        width: 1.0,
        paint: label.with_alpha(110).into(),
    });
    items.push(Item::Fill {
        shape: Shape::rounded(
            Rect::new(
                x + BATTERY_WIDTH + 0.5,
                y + BATTERY_HEIGHT / 2.0 - 2.0,
                1.5,
                4.0,
            ),
            0.75,
        ),
        paint: label.with_alpha(110).into(),
    });
    let inner = body.inset(2.0);
    let level = (battery.percent / 100.0).clamp(0.0, 1.0) as f32;
    let fill = if battery.low() {
        palette.danger_text
    } else if battery.on_adapter() {
        palette.success
    } else {
        label
    };
    items.push(Item::Fill {
        shape: Shape::rounded(
            Rect::new(
                inner.x,
                inner.y,
                (inner.width * level).max(1.5),
                inner.height,
            ),
            1.5,
        ),
        paint: fill.into(),
    });
    if battery.on_adapter() {
        let halo = palette.overlay;
        let (width, height) = natural_size(Symbol::Bolt);
        for (color, offset) in [(halo, -0.6), (halo, 0.6), (label, 0.0)] {
            if let Some(image) = symbols.image(Symbol::Bolt, color, scale) {
                items.push(Item::Image {
                    rect: Rect::new(
                        snap(body.x + (BATTERY_WIDTH - width) / 2.0, scale) + offset,
                        snap(body.y + (BATTERY_HEIGHT - height) / 2.0, scale),
                        image.width() as f32 / scale as f32,
                        image.height() as f32 / scale as f32,
                    ),
                    image,
                });
            }
        }
    }
}
