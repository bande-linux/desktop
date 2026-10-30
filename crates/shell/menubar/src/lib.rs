pub mod dbusmenu;
pub mod devices;
pub mod menus;
pub mod session;
pub mod status;
mod symbols;
pub mod tray;

use std::sync::LazyLock;

use unbound_l10n::Localizer;
use unbound_toolkit::ui::Menu;
use unbound_toolkit::{
    Color, Image, Item, Palette, Paragraph, Point, Rect, Scene, Shape, TextAlign, TextStyle,
    TextSystem,
};

use crate::dbusmenu::Node;
use crate::status::{Battery, Status, speaker_waves, wifi_bars};
pub use crate::symbols::Symbol;
use crate::symbols::{Symbols, natural_size};
use crate::tray::TrayItem;

const ENGLISH: &str = include_str!("../locales/en-US/unbound-menubar.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub const HEIGHT: u32 = 24;
const EDGE: f32 = 8.0;
const TEXT_SIZE: f32 = 13.0;
const TEXT_PADDING: f32 = 9.0;
const SYMBOL_SLOT: f32 = 30.0;
const TRAY_SLOT: f32 = 28.0;
const TRAY_ICON: f32 = 16.0;
const BATTERY_SLOT: f32 = 38.0;
const BATTERY_WIDTH: f32 = 23.0;
const BATTERY_HEIGHT: f32 = 11.5;
const HIGHLIGHT_INSET: f32 = 2.0;
const TRANSLUCENT_ALPHA: u8 = 176;
const OPAQUE_ALPHA: u8 = 240;
const LOW: Color = Color::hex(0xFF453A);
const CHARGING: Color = Color::hex(0x30D158);

pub fn clock_text(now: &jiff::Zoned) -> String {
    now.strftime("%a %-d %b %H:%M").to_string()
}

pub fn app_name<'a>(app_id: &'a str, names: &'a [(String, String)]) -> &'a str {
    names
        .iter()
        .find(|(id, _)| id == app_id)
        .map_or(app_id, |(_, name)| name.as_str())
}

#[derive(Debug, Clone)]
pub enum Update {
    Devices(devices::Devices),
    Sound(Option<status::Sound>),
    Tray(Vec<TrayItem>),
    Menu { menu: Menu, anchor: Rect },
    AppMenu { app: String, root: Option<Node> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    System,
    App,
    AppMenu(usize),
    Tray(usize),
    Bluetooth,
    Sound,
    Wifi,
    Battery,
    ControlCenter,
    Clock,
}

pub struct MenuBar {
    palette: Palette,
    text: TextSystem,
    symbols: Symbols,
    app: String,
    app_menus: Vec<String>,
    tray: Vec<Option<Image>>,
    status: Status,
    clock: String,
    open: Option<Slot>,
    slots: Vec<(Slot, Rect)>,
    translucent: bool,
}

fn snap(value: f32, scale: f64) -> f32 {
    ((f64::from(value) * scale).round() / scale) as f32
}

fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x
        && point.x < rect.x + rect.width
        && point.y >= rect.y
        && point.y < rect.y + rect.height
}

impl MenuBar {
    pub fn new(palette: Palette, text: TextSystem, clock: String) -> Self {
        Self {
            palette,
            text,
            symbols: Symbols::default(),
            app: String::new(),
            app_menus: Vec::new(),
            tray: Vec::new(),
            status: Status::default(),
            clock,
            open: None,
            slots: Vec::new(),
            translucent: false,
        }
    }

    pub fn set_app(&mut self, name: &str) -> bool {
        let changed = self.app != name;
        name.clone_into(&mut self.app);
        changed
    }

    pub fn app(&self) -> &str {
        &self.app
    }

    pub fn set_app_menus(&mut self, titles: Vec<String>) {
        self.app_menus = titles;
    }

    pub fn set_tray(&mut self, icons: Vec<Option<Image>>) {
        self.tray = icons;
    }

    pub fn status_mut(&mut self) -> &mut Status {
        &mut self.status
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn set_translucent(&mut self, translucent: bool) {
        self.translucent = translucent;
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    pub fn set_clock(&mut self, clock: String) -> bool {
        let changed = self.clock != clock;
        self.clock = clock;
        changed
    }

    pub fn set_open(&mut self, slot: Option<Slot>) -> bool {
        let changed = self.open != slot;
        self.open = slot;
        changed
    }

    pub fn open(&self) -> Option<Slot> {
        self.open
    }

    pub fn slot_at(&self, point: Point) -> Option<Slot> {
        self.slots
            .iter()
            .find(|(_, rect)| contains(*rect, point))
            .map(|(slot, _)| *slot)
    }

    pub fn slot_rect(&self, slot: Slot) -> Option<Rect> {
        self.slots
            .iter()
            .find(|(found, _)| *found == slot)
            .map(|(_, rect)| *rect)
    }

    fn label(&mut self, text: &str, bold: bool) -> Paragraph {
        let style = TextStyle::new(TEXT_SIZE, self.palette.label_primary);
        let style = if bold {
            style.weight(TextStyle::SEMIBOLD)
        } else {
            style
        };
        self.text.paragraph(text, style, None, TextAlign::Start)
    }

    fn right_slots(&self) -> Vec<(Slot, f32)> {
        let mut slots: Vec<(Slot, f32)> = (0..self.tray.len())
            .map(|index| (Slot::Tray(index), TRAY_SLOT))
            .collect();
        if self.status.bluetooth.is_some() {
            slots.push((Slot::Bluetooth, SYMBOL_SLOT - 4.0));
        }
        if self.status.sound.is_some() {
            slots.push((Slot::Sound, SYMBOL_SLOT));
        }
        if self.status.wifi.is_some() {
            slots.push((Slot::Wifi, SYMBOL_SLOT));
        }
        if self.status.battery.is_some() {
            slots.push((Slot::Battery, BATTERY_SLOT));
        }
        slots.push((Slot::ControlCenter, SYMBOL_SLOT));
        slots
    }

    fn symbol_of(&self, slot: Slot) -> Option<Symbol> {
        Some(match slot {
            Slot::System => Symbol::Logo,
            Slot::ControlCenter => Symbol::ControlCenter,
            Slot::Bluetooth => match &self.status.bluetooth {
                Some(bluetooth) if bluetooth.powered => Symbol::Bluetooth,
                _ => Symbol::BluetoothOff,
            },
            Slot::Sound => match &self.status.sound {
                Some(sound) if sound.muted => Symbol::SpeakerMuted,
                Some(sound) => Symbol::Speaker {
                    waves: speaker_waves(sound.volume),
                },
                None => return None,
            },
            Slot::Wifi => match &self.status.wifi {
                Some(wifi) if wifi.enabled => Symbol::Wifi {
                    bars: wifi
                        .current()
                        .map_or(0, |network| wifi_bars(network.strength)),
                },
                _ => Symbol::WifiOff,
            },
            _ => return None,
        })
    }

    fn place_symbol(&mut self, symbol: Symbol, rect: Rect, scale: f64, items: &mut Vec<Item>) {
        let (width, height) = natural_size(symbol);
        let Some(image) = self
            .symbols
            .image(symbol, self.palette.label_primary, scale)
        else {
            return;
        };
        let x = snap(rect.x + (rect.width - width) / 2.0, scale);
        let y = snap(rect.y + (rect.height - height) / 2.0, scale);
        items.push(Item::Image {
            rect: Rect::new(
                x,
                y,
                image.width() as f32 / scale as f32,
                image.height() as f32 / scale as f32,
            ),
            image,
        });
    }

    fn paint_battery(&mut self, battery: Battery, rect: Rect, scale: f64, items: &mut Vec<Item>) {
        let label = self.palette.label_primary;
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
            LOW
        } else if battery.on_adapter() {
            CHARGING
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
            let halo = self.palette.surface_elevated;
            let bolt = Rect::new(body.x, body.y, BATTERY_WIDTH, BATTERY_HEIGHT);
            for (color, offset) in [(halo, -0.6), (halo, 0.6), (label, 0.0)] {
                if let Some(image) = self.symbols.image(Symbol::Bolt, color, scale) {
                    let (width, height) = natural_size(Symbol::Bolt);
                    items.push(Item::Image {
                        rect: Rect::new(
                            snap(bolt.x + (bolt.width - width) / 2.0, scale) + offset,
                            snap(bolt.y + (bolt.height - height) / 2.0, scale),
                            image.width() as f32 / scale as f32,
                            image.height() as f32 / scale as f32,
                        ),
                        image,
                    });
                }
            }
        }
    }

    pub fn scene(&mut self, width: u32, height: u32, scale: f64) -> Scene {
        let bar = Rect::new(0.0, 0.0, width as f32, height as f32);
        let mut items = vec![
            Item::Fill {
                shape: Shape::rect(bar),
                paint: self
                    .palette
                    .surface_elevated
                    .with_alpha(if self.translucent {
                        TRANSLUCENT_ALPHA
                    } else {
                        OPAQUE_ALPHA
                    })
                    .into(),
            },
            Item::Fill {
                shape: Shape::rect(Rect::new(0.0, bar.height - 1.0, bar.width, 1.0)),
                paint: self.palette.separator_color.with_alpha(160).into(),
            },
        ];
        let mut slots = Vec::new();
        let full = |x: f32, width: f32| Rect::new(x, 0.0, width, bar.height);
        let mut x = EDGE;
        slots.push((Slot::System, full(x, SYMBOL_SLOT)));
        x += SYMBOL_SLOT;
        let mut texts = Vec::new();
        if !self.app.is_empty() {
            let app = self.app.clone();
            let paragraph = self.label(&app, true);
            let width = paragraph.width() + 2.0 * TEXT_PADDING;
            slots.push((Slot::App, full(x, width)));
            texts.push((x + TEXT_PADDING, paragraph));
            x += width;
        }
        for (index, title) in self.app_menus.clone().iter().enumerate() {
            let paragraph = self.label(title, false);
            let width = paragraph.width() + 2.0 * TEXT_PADDING;
            slots.push((Slot::AppMenu(index), full(x, width)));
            texts.push((x + TEXT_PADDING, paragraph));
            x += width;
        }
        let clock = self.clock.clone();
        let clock = self.label(&clock, false);
        let clock_width = clock.width() + 2.0 * TEXT_PADDING;
        let mut right = bar.width - EDGE - clock_width;
        slots.push((Slot::Clock, full(right, clock_width)));
        texts.push((right + TEXT_PADDING, clock));
        for (slot, width) in self.right_slots().into_iter().rev() {
            right -= width;
            slots.push((slot, full(right, width)));
        }
        if let Some(open) = self.open
            && let Some((_, rect)) = slots.iter().find(|(slot, _)| *slot == open)
        {
            items.push(Item::Fill {
                shape: Shape::rounded(
                    Rect::new(
                        rect.x,
                        HIGHLIGHT_INSET,
                        rect.width,
                        bar.height - 2.0 * HIGHLIGHT_INSET - 1.0,
                    ),
                    5.0,
                ),
                paint: self.palette.label_primary.with_alpha(34).into(),
            });
        }
        for (x, paragraph) in texts {
            let y = snap((bar.height - 1.0 - paragraph.height()) / 2.0, scale);
            items.push(Item::Text {
                origin: Point::new(snap(x, scale), y),
                paragraph,
            });
        }
        for (slot, rect) in slots.clone() {
            let content = Rect::new(rect.x, rect.y, rect.width, rect.height - 1.0);
            match slot {
                Slot::Battery => {
                    if let Some(battery) = self.status.battery {
                        self.paint_battery(battery, content, scale, &mut items);
                    }
                }
                Slot::Tray(index) => {
                    if let Some(Some(image)) = self.tray.get(index).cloned() {
                        items.push(Item::Image {
                            rect: Rect::new(
                                snap(content.x + (content.width - TRAY_ICON) / 2.0, scale),
                                snap(content.y + (content.height - TRAY_ICON) / 2.0, scale),
                                TRAY_ICON,
                                TRAY_ICON,
                            ),
                            image,
                        });
                    }
                }
                slot => {
                    if let Some(symbol) = self.symbol_of(slot) {
                        self.place_symbol(symbol, content, scale, &mut items);
                    }
                }
            }
        }
        self.slots = slots;
        Scene {
            width,
            height,
            background: Color::TRANSPARENT,
            items,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("lib.rs"),
            include_str!("menus.rs"),
            include_str!("session.rs"),
            include_str!("main.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
