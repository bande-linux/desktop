use std::collections::HashMap;
use std::path::PathBuf;

use accesskit::{Action, Node as AccessNode, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
use bande_datetime::DateFormats;
use bande_status::status::{Battery, Capture, Sense, Status, speaker_waves};
use bande_status::unread::ClockMark;
use bande_toolkit::ui::PointerButton;
use bande_toolkit::{
    Color, DisplayEdge, Image, Item, MenuPlacement, Palette, Paragraph, Point, Rect, Scene, Shape,
    TextAlign, TextStyle, TextSystem,
};
use bande_xdg::{ICON_THEMES, find_icon};
use jiff::Zoned;

use crate::L10N;
use crate::dock::{Rail as AppsRegion, WELL_ALPHA};
use crate::menu_bar::{Slot, StatusView};

pub const EDGE_PADDING: f32 = 24.0;
pub const LABEL_ROOM: f32 = 280.0;
const MARK_CELL: f32 = 52.0;
const HEADER_CELL: f32 = 58.0;
const HEADER_WIDTH: f32 = 52.0;
const HEADER_ICON: f32 = 24.0;
const CAPTION_SIZE: f32 = 11.0;
const DIMMED: f32 = 0.45;
const MARK_SIZE: f32 = 22.0;
const CLOCK_CELL: f32 = 52.0;
const STATUS_WIDTH: f32 = 52.0;
const STATUS_HEIGHT: f32 = 58.0;
const GRID_CELL: f32 = 26.0;
const GLYPH: f32 = 15.0;
const GROUP_GAP: f32 = 6.0;
const GROUP_RADIUS: f32 = 8.0;
const HAIRLINE: f32 = 32.0;
const HAIRLINE_CELL: f32 = 13.0;
const WORKSPACE_DOT: f32 = 6.0;
const WORKSPACE_PILL: f32 = 12.0;
const WORKSPACE_CELL: f32 = 10.0;
const TRAY_SHOWN: usize = 3;
const TIME_SIZE: f32 = 15.0;
const DAY_SIZE: f32 = 11.0;
const BADGE_SIZE: f32 = 10.0;
const PERCENT_SIZE: f32 = 8.0;
const UNREAD_DOT: f32 = 7.0;
const IN_USE_DOT: f32 = 6.0;
const HOVER_ALPHA: u8 = 18;
const OPEN_ALPHA: u8 = 30;
const NODE_BASE: u64 = 1 << 40;
const CONTROL_CENTER_GLYPH: &str = "bande-control-center-symbolic";
const MARK_GLYPH: &str = "bande-symbolic";
const SEARCH_GLYPH: &str = "system-search-symbolic";
pub const EXPANDED_WIDTH: f32 = 248.0;
const ROW_INSET: f32 = 8.0;
const SEARCH_HEIGHT: f32 = 32.0;
const HEADER_ROW: f32 = 36.0;
const MENU_ROW: f32 = 28.0;
const MENU_TEXT: f32 = 13.0;
const EXPANDED_CLOCK: f32 = 64.0;
const EXPANDED_TIME: f32 = 22.0;
const EXPANDED_STATUS: f32 = 52.0;
const PILL: (f32, f32) = (26.0, 20.0);
const EXPANDED_TRAY: usize = 6;

pub fn bar_width(icon_size: f32) -> f32 {
    icon_size + EDGE_PADDING
}

pub fn surface_size(icon_size: f32) -> (u32, u32) {
    ((bar_width(icon_size) + LABEL_ROOM).ceil() as u32, 0)
}

pub fn owns_node(node: NodeId) -> bool {
    node.0 >= NODE_BASE - 1
}

struct Clock {
    time: String,
    period: Option<String>,
    day: String,
    long: String,
}

pub struct RailView {
    palette: Palette,
    text: TextSystem,
    icon_dirs: Vec<PathBuf>,
    icons: HashMap<(String, Color, u32), Option<Image>>,
    width: f32,
    icon_size: f32,
    expanded: bool,
    menus: Vec<String>,
    app: String,
    app_icon: Option<Image>,
    key_display: bool,
    tray: Vec<(String, Option<Image>)>,
    status: Status,
    clock: Clock,
    clock_mark: ClockMark,
    battery_percentage: bool,
    workspaces: (usize, usize),
    open: Option<Slot>,
    hovered: Option<Slot>,
    slots: Vec<(Slot, Rect)>,
}

pub fn in_use(capturing: &[Capture]) -> Vec<String> {
    capturing
        .iter()
        .map(|capture| {
            let id = match capture.senses.as_slice() {
                [Sense::Microphone] => "in-use-microphone",
                [Sense::Camera] => "in-use-camera",
                _ => "in-use-microphone-and-camera",
            };
            L10N.tr_args(id, [("app", capture.name.clone().into())])
        })
        .collect()
}

pub fn control_center_label(status: &Status) -> String {
    let mut parts = vec![L10N.tr("control-center")];
    parts.extend(in_use(&status.capturing));
    parts.join(", ")
}

pub fn in_use_dot(rect: Rect, palette: &Palette) -> Item {
    Item::Fill {
        shape: Shape::rounded(
            Rect::new(
                rect.x + rect.width - IN_USE_DOT - 2.0,
                rect.y + 2.0,
                IN_USE_DOT,
                IN_USE_DOT,
            ),
            IN_USE_DOT / 2.0,
        ),
        paint: palette.accent.into(),
    }
}

fn snap(value: f32, scale: f64) -> f32 {
    ((f64::from(value) * scale).round() / scale) as f32
}

fn battery_glyph(battery: Battery) -> String {
    let level = ((battery.percent / 10.0).round() * 10.0).clamp(0.0, 100.0) as u32;
    let suffix = match battery.power {
        bande_status::status::Power::Full if level == 100 => "-charged",
        bande_status::status::Power::Charging { .. } => "-charging",
        _ => "",
    };
    format!("battery-level-{level}{suffix}-symbolic")
}

fn wifi_glyph(status: &Status) -> Option<String> {
    let wifi = status.wifi.as_ref()?;
    let name = match wifi.current() {
        _ if !wifi.enabled => "network-wireless-disabled",
        None => "network-wireless-offline",
        Some(network) => match network.strength {
            0..25 => "network-wireless-signal-weak",
            25..50 => "network-wireless-signal-ok",
            50..75 => "network-wireless-signal-good",
            _ => "network-wireless-signal-excellent",
        },
    };
    Some(format!("{name}-symbolic"))
}

fn sound_glyph(status: &Status) -> Option<String> {
    let sound = status.sound.as_ref()?;
    let name = match speaker_waves(sound.volume) {
        _ if sound.muted => "muted",
        0 => "muted",
        1 => "low",
        2 => "medium",
        _ => "high",
    };
    Some(format!("audio-volume-{name}-symbolic"))
}

fn bluetooth_glyph(status: &Status) -> Option<String> {
    let bluetooth = status.bluetooth.as_ref()?;
    Some(
        if !bluetooth.powered {
            "bluetooth-disabled-symbolic"
        } else if bluetooth.devices.iter().any(|gadget| gadget.connected) {
            "bluetooth-active-symbolic"
        } else {
            "bluetooth-symbolic"
        }
        .to_owned(),
    )
}

impl RailView {
    pub fn new(
        palette: Palette,
        text: TextSystem,
        icon_dirs: Vec<PathBuf>,
        icon_size: f32,
    ) -> Self {
        Self {
            palette,
            text,
            icon_dirs,
            icons: HashMap::new(),
            width: bar_width(icon_size),
            icon_size,
            expanded: false,
            menus: Vec::new(),
            app: String::new(),
            app_icon: None,
            key_display: true,
            tray: Vec::new(),
            status: Status::default(),
            clock: Clock {
                time: String::new(),
                period: None,
                day: String::new(),
                long: String::new(),
            },
            clock_mark: ClockMark::Clear,
            battery_percentage: false,
            workspaces: (1, 0),
            open: None,
            hovered: None,
            slots: Vec::new(),
        }
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn unread(&self) -> bool {
        self.clock_mark == ClockMark::Unread
    }

    pub fn set_icon_size(&mut self, icon_size: f32) {
        self.icon_size = icon_size;
        self.follow_width();
    }

    pub fn set_expanded(&mut self, expanded: bool) -> bool {
        let changed = self.expanded != expanded;
        self.expanded = expanded;
        self.follow_width();
        changed
    }

    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    fn follow_width(&mut self) {
        self.width = if self.expanded {
            EXPANDED_WIDTH
        } else {
            bar_width(self.icon_size)
        };
    }

    fn tray_split(&self) -> (Vec<Slot>, Vec<Slot>) {
        let mut cells: Vec<Slot> = (0..self.tray.len()).map(Slot::Tray).collect();
        if self.status.input_sources.current().is_some() {
            cells.push(Slot::InputSource);
        }
        let shown = if self.expanded {
            EXPANDED_TRAY
        } else {
            TRAY_SHOWN
        };
        let hidden = cells.split_off(shown.min(cells.len()));
        (cells, hidden)
    }

    fn tray_cells(&self) -> Vec<Slot> {
        let (mut cells, hidden) = self.tray_split();
        if !hidden.is_empty() {
            cells.push(Slot::TrayMore);
        }
        cells
    }

    fn glyph_slots(&self) -> [(Slot, Option<String>); 4] {
        [
            (Slot::Wifi, wifi_glyph(&self.status)),
            (Slot::Sound, sound_glyph(&self.status)),
            (Slot::Bluetooth, bluetooth_glyph(&self.status)),
            (Slot::Battery, self.status.battery.map(battery_glyph)),
        ]
    }

    fn layout(&self, height: f32) -> (Vec<(Slot, Rect)>, AppsRegion, Vec<f32>) {
        if self.expanded {
            return self.layout_expanded(height);
        }
        let width = self.width;
        let mut slots = vec![(Slot::System, Rect::new(0.0, 0.0, width, MARK_CELL))];
        if !self.app.is_empty() {
            slots.push((
                Slot::App,
                Rect::new(
                    ((width - HEADER_WIDTH) / 2.0).round(),
                    MARK_CELL,
                    HEADER_WIDTH,
                    HEADER_CELL,
                ),
            ));
        }
        let mut hairlines = Vec::new();
        let mut bottom = height - CLOCK_CELL;
        slots.push((Slot::Clock, Rect::new(0.0, bottom, width, CLOCK_CELL)));
        let group_x = ((width - STATUS_WIDTH) / 2.0).round();
        bottom -= STATUS_HEIGHT + GROUP_GAP;
        let status = Rect::new(group_x, bottom, STATUS_WIDTH, STATUS_HEIGHT);
        slots.push((Slot::ControlCenter, status));
        let row = STATUS_HEIGHT / 2.0;
        for (index, (slot, _)) in self.glyph_slots().into_iter().enumerate() {
            let column = (index % 2) as f32;
            let line = (index / 2) as f32;
            slots.push((
                slot,
                Rect::new(
                    group_x + column * GRID_CELL,
                    bottom + line * row,
                    GRID_CELL,
                    row,
                ),
            ));
        }
        let tray = self.tray_cells();
        if !tray.is_empty() {
            let rows = tray.len().div_ceil(2);
            bottom -= GROUP_GAP + rows as f32 * GRID_CELL;
            for (index, slot) in tray.into_iter().enumerate() {
                let column = (index % 2) as f32;
                let line = (index / 2) as f32;
                slots.push((
                    slot,
                    Rect::new(
                        group_x + column * GRID_CELL,
                        bottom + line * GRID_CELL,
                        GRID_CELL,
                        GRID_CELL,
                    ),
                ));
            }
        }
        let (count, _) = self.workspaces;
        if count >= 2 {
            bottom -= GROUP_GAP + count as f32 * WORKSPACE_CELL + (WORKSPACE_PILL - WORKSPACE_DOT);
            let mut y = bottom;
            for index in 0..count {
                let length = if index == self.workspaces.1 {
                    WORKSPACE_PILL
                } else {
                    WORKSPACE_DOT
                } + (WORKSPACE_CELL - WORKSPACE_DOT);
                slots.push((Slot::Workspace(index), Rect::new(0.0, y, width, length)));
                y += length;
            }
        }
        bottom -= HAIRLINE_CELL;
        hairlines.push(bottom + HAIRLINE_CELL / 2.0);
        let top = MARK_CELL + HEADER_CELL + HAIRLINE_CELL;
        hairlines.push(MARK_CELL + HEADER_CELL + HAIRLINE_CELL / 2.0);
        let region = AppsRegion {
            top,
            bottom: bottom.max(top),
            width,
            expanded: false,
        };
        (slots, region, hairlines)
    }

    fn layout_expanded(&self, height: f32) -> (Vec<(Slot, Rect)>, AppsRegion, Vec<f32>) {
        let width = self.width;
        let inner = width - 2.0 * ROW_INSET;
        let mut slots = vec![
            (Slot::System, Rect::new(0.0, 0.0, MARK_CELL, MARK_CELL)),
            (
                Slot::Launcher,
                Rect::new(
                    MARK_CELL,
                    (MARK_CELL - SEARCH_HEIGHT) / 2.0,
                    width - MARK_CELL - ROW_INSET,
                    SEARCH_HEIGHT,
                ),
            ),
        ];
        let mut top = MARK_CELL;
        if !self.app.is_empty() {
            slots.push((Slot::App, Rect::new(ROW_INSET, top, inner, HEADER_ROW)));
            top += HEADER_ROW;
            for index in 0..self.menus.len() {
                slots.push((
                    Slot::AppMenu(index),
                    Rect::new(ROW_INSET, top, inner, MENU_ROW),
                ));
                top += MENU_ROW;
            }
            top += GROUP_GAP;
        }
        let mut hairlines = vec![top + HAIRLINE_CELL / 2.0];
        top += HAIRLINE_CELL;
        let mut bottom = height - EXPANDED_CLOCK;
        slots.push((Slot::Clock, Rect::new(0.0, bottom, width, EXPANDED_CLOCK)));
        bottom -= EXPANDED_STATUS + GROUP_GAP;
        slots.push((
            Slot::ControlCenter,
            Rect::new(ROW_INSET, bottom, inner, EXPANDED_STATUS),
        ));
        for (index, (slot, _)) in self.glyph_slots().into_iter().enumerate() {
            slots.push((
                slot,
                Rect::new(
                    2.0 * ROW_INSET + index as f32 * GRID_CELL,
                    bottom + 4.0,
                    GRID_CELL,
                    GRID_CELL,
                ),
            ));
        }
        let tray = self.tray_cells();
        if !tray.is_empty() {
            bottom -= GROUP_GAP + GRID_CELL;
            for (index, slot) in tray.into_iter().enumerate() {
                slots.push((
                    slot,
                    Rect::new(
                        2.0 * ROW_INSET + index as f32 * GRID_CELL,
                        bottom,
                        GRID_CELL,
                        GRID_CELL,
                    ),
                ));
            }
        }
        let (count, _) = self.workspaces;
        if count >= 2 {
            bottom -= GROUP_GAP + GRID_CELL;
            for index in 0..count {
                slots.push((
                    Slot::Workspace(index),
                    Rect::new(
                        2.0 * ROW_INSET + index as f32 * (PILL.0 + 4.0),
                        bottom + (GRID_CELL - PILL.1) / 2.0,
                        PILL.0,
                        PILL.1,
                    ),
                ));
            }
        }
        bottom -= HAIRLINE_CELL;
        hairlines.push(bottom + HAIRLINE_CELL / 2.0);
        let region = AppsRegion {
            top,
            bottom: bottom.max(top),
            width,
            expanded: true,
        };
        (slots, region, hairlines)
    }

    pub fn apps_region(&self, height: f32) -> AppsRegion {
        self.layout(height).1
    }

    fn glyph(&mut self, name: &str, color: Color, scale: f64) -> Option<Image> {
        let pixels = (f64::from(GLYPH) * scale).round() as u32;
        let dirs = &self.icon_dirs;
        self.icons
            .entry((name.to_owned(), color, pixels))
            .or_insert_with(|| {
                let path = find_icon(name, pixels, &ICON_THEMES, dirs)?;
                Image::load_symbolic(&path, pixels, color)
            })
            .clone()
    }

    fn place_image(rect: Rect, image: Image, size: f32, scale: f64, items: &mut Vec<Item>) {
        items.push(Item::Image {
            rect: Rect::new(
                snap(rect.x + (rect.width - size) / 2.0, scale),
                snap(rect.y + (rect.height - size) / 2.0, scale),
                size,
                size,
            ),
            image,
        });
    }

    fn paragraph(&mut self, text: &str, style: TextStyle) -> Paragraph {
        self.text.paragraph(text, style, None, TextAlign::Start)
    }

    fn slot_alpha(&self, slot: Slot) -> Option<u8> {
        if self.open == Some(slot) {
            Some(OPEN_ALPHA)
        } else if self.hovered == Some(slot) {
            Some(HOVER_ALPHA)
        } else {
            None
        }
    }

    fn highlight(&self, slot: Slot, rect: Rect, items: &mut Vec<Item>) {
        let Some(alpha) = self.slot_alpha(slot) else {
            return;
        };
        let inset = match slot {
            Slot::System | Slot::Clock => Rect::new(
                rect.x + 6.0,
                rect.y + 4.0,
                rect.width - 12.0,
                rect.height - 8.0,
            ),
            Slot::Workspace(_) | Slot::App => return,
            _ => rect,
        };
        items.push(Item::Fill {
            shape: Shape::rounded(inset, GROUP_RADIUS),
            paint: self.palette.ink.with_alpha(alpha).into(),
        });
    }

    fn paint(&mut self, width: f32, height: f32, scale: f64) -> Vec<Item> {
        let (slots, _, hairlines) = self.layout(height);
        let palette = self.palette;
        let ink = palette.ink;
        let mut items = vec![
            Item::Fill {
                shape: Shape::rect(Rect::new(0.0, 0.0, width, height)),
                paint: palette.chassis.into(),
            },
            Item::Fill {
                shape: Shape::rect(Rect::new(width - 1.0, 0.0, 1.0, height)),
                paint: palette.edge.into(),
            },
        ];
        let hairline = if self.expanded {
            self.width - 4.0 * ROW_INSET
        } else {
            HAIRLINE
        };
        for y in hairlines {
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    ((self.width - hairline) / 2.0).round(),
                    (y - 0.5).round(),
                    hairline,
                    1.0,
                )),
                paint: palette.hairline.into(),
            });
        }
        let status_rect = slots
            .iter()
            .find(|(slot, _)| *slot == Slot::ControlCenter)
            .map(|(_, rect)| *rect);
        if let Some(rect) = status_rect {
            items.push(Item::Fill {
                shape: Shape::rounded(rect, GROUP_RADIUS),
                paint: ink.with_alpha(WELL_ALPHA).into(),
            });
        }
        for (slot, rect) in &slots {
            self.highlight(*slot, *rect, &mut items);
        }
        let mut placeholder = Some(CONTROL_CENTER_GLYPH.to_owned());
        let glyphs = self.glyph_slots();
        for (slot, rect) in slots.clone() {
            match slot {
                Slot::System => {
                    let pixels = MARK_SIZE;
                    let image = {
                        let size = (f64::from(pixels) * scale).round() as u32;
                        let dirs = &self.icon_dirs;
                        self.icons
                            .entry((MARK_GLYPH.to_owned(), ink, size))
                            .or_insert_with(|| {
                                let path = find_icon(MARK_GLYPH, size, &ICON_THEMES, dirs)?;
                                Image::load_symbolic(&path, size, ink)
                            })
                            .clone()
                    };
                    if let Some(image) = image {
                        Self::place_image(rect, image, pixels, scale, &mut items);
                    }
                }
                Slot::Wifi | Slot::Sound | Slot::Bluetooth | Slot::Battery => {
                    let name = glyphs
                        .iter()
                        .find(|(known, _)| *known == slot)
                        .and_then(|(_, name)| name.clone())
                        .or_else(|| placeholder.take());
                    let percent = self
                        .status
                        .battery
                        .filter(|_| slot == Slot::Battery && self.battery_percentage)
                        .map(|battery| {
                            let text = L10N.tr_args(
                                "battery-percent",
                                [("percent", battery.percent.round().into())],
                            );
                            self.paragraph(
                                &text,
                                TextStyle::new(PERCENT_SIZE, ink)
                                    .weight(TextStyle::SEMIBOLD)
                                    .tabular(),
                            )
                        });
                    let glyph_rect = match &percent {
                        Some(label) => Rect::new(
                            rect.x,
                            rect.y + (rect.height - GLYPH - label.height()) / 2.0,
                            rect.width,
                            GLYPH,
                        ),
                        None => rect,
                    };
                    if let Some(image) = name.and_then(|name| self.glyph(&name, ink, scale)) {
                        Self::place_image(glyph_rect, image, GLYPH, scale, &mut items);
                    }
                    if let Some(label) = percent {
                        items.push(Item::Text {
                            origin: Point::new(
                                snap(rect.x + (rect.width - label.width()) / 2.0, scale),
                                snap(glyph_rect.y + GLYPH, scale),
                            ),
                            paragraph: label,
                        });
                    }
                }
                Slot::Tray(index) => {
                    if let Some((_, Some(image))) = self.tray.get(index).cloned() {
                        Self::place_image(rect, image, GLYPH, scale, &mut items);
                    }
                }
                Slot::InputSource => {
                    if let Some(source) = self.status.input_sources.current() {
                        let short = source.short.clone();
                        let text = self.paragraph(
                            &short,
                            TextStyle::new(BADGE_SIZE, ink).weight(TextStyle::SEMIBOLD),
                        );
                        let badge = Rect::new(
                            snap(rect.x + (rect.width - text.width() - 6.0) / 2.0, scale),
                            snap(rect.y + (rect.height - 14.0) / 2.0, scale),
                            (text.width() + 6.0).round(),
                            14.0,
                        );
                        items.push(Item::Stroke {
                            shape: Shape::rounded(badge.inset(0.5), 3.0),
                            width: 1.0,
                            paint: ink.into(),
                        });
                        items.push(Item::Text {
                            origin: Point::new(
                                snap(badge.x + 3.0, scale),
                                snap(badge.y + (badge.height - text.height()) / 2.0, scale),
                            ),
                            paragraph: text,
                        });
                    }
                }
                Slot::TrayMore => {
                    let text = self.paragraph("•••", TextStyle::new(BADGE_SIZE, ink));
                    items.push(Item::Text {
                        origin: Point::new(
                            snap(rect.x + (rect.width - text.width()) / 2.0, scale),
                            snap(rect.y + (rect.height - text.height()) / 2.0, scale),
                        ),
                        paragraph: text,
                    });
                }
                Slot::Workspace(index) if self.expanded => {
                    self.paint_pill(rect, index, &mut items);
                }
                Slot::Launcher => self.paint_search(rect, scale, &mut items),
                Slot::AppMenu(index) => self.paint_menu_row(rect, index, &mut items),
                Slot::Workspace(index) => {
                    let active = index == self.workspaces.1;
                    let length = if active {
                        WORKSPACE_PILL
                    } else {
                        WORKSPACE_DOT
                    };
                    let dot = Rect::new(
                        ((width - WORKSPACE_DOT) / 2.0).round(),
                        (rect.y + (rect.height - length) / 2.0).round(),
                        WORKSPACE_DOT,
                        length,
                    );
                    items.push(Item::Fill {
                        shape: Shape::rounded(dot, WORKSPACE_DOT / 2.0),
                        paint: if active { ink } else { palette.ink2 }.into(),
                    });
                }
                Slot::Clock if self.expanded => self.paint_long_clock(rect, scale, &mut items),
                Slot::ControlCenter if self.expanded => self.paint_summary(rect, &mut items),
                Slot::Clock => self.paint_clock(rect, scale, &mut items),
                Slot::App => self.paint_header(rect, scale, &mut items),
                _ => {}
            }
            if slot == Slot::ControlCenter && !self.status.capturing.is_empty() {
                items.push(in_use_dot(rect, &palette));
            }
        }
        self.slots = slots;
        items
    }

    fn fitted(&mut self, text: &str, style: TextStyle, width: f32) -> Paragraph {
        let mut characters: Vec<char> = text.chars().collect();
        let mut shown = text.to_owned();
        loop {
            let paragraph = self.paragraph(&shown, style);
            if paragraph.width() <= width || characters.len() <= 1 {
                return paragraph;
            }
            characters.pop();
            shown = format!("{}…", characters.iter().collect::<String>().trim_end());
        }
    }

    fn centered_text(rect: Rect, x: f32, paragraph: Paragraph, items: &mut Vec<Item>) {
        items.push(Item::Text {
            origin: Point::new(
                x.round(),
                (rect.y + (rect.height - paragraph.height()) / 2.0).round(),
            ),
            paragraph,
        });
    }

    fn paint_search(&mut self, rect: Rect, scale: f64, items: &mut Vec<Item>) {
        let palette = self.palette;
        items.push(Item::Fill {
            shape: Shape::rounded(rect, GROUP_RADIUS),
            paint: palette.well.into(),
        });
        if let Some(glyph) = self.glyph(SEARCH_GLYPH, palette.ink2, scale) {
            Self::place_image(
                Rect::new(rect.x + 6.0, rect.y, GRID_CELL, rect.height),
                glyph,
                GLYPH,
                scale,
                items,
            );
        }
        let hint = self.paragraph(
            &L10N.tr("search-shortcut"),
            TextStyle::new(CAPTION_SIZE, palette.ink2),
        );
        let hint_x = rect.x + rect.width - 10.0 - hint.width();
        let label = self.paragraph(&L10N.tr("search"), TextStyle::new(MENU_TEXT, palette.ink2));
        Self::centered_text(rect, rect.x + 8.0 + GRID_CELL, label, items);
        Self::centered_text(rect, hint_x, hint, items);
    }

    fn paint_menu_row(&mut self, rect: Rect, index: usize, items: &mut Vec<Item>) {
        let Some(title) = self.menus.get(index).cloned() else {
            return;
        };
        let palette = self.palette;
        let mut row = Vec::new();
        let chevron = self.paragraph("›", TextStyle::new(MENU_TEXT, palette.ink2));
        let chevron_x = rect.x + rect.width - 10.0 - chevron.width();
        let text_x = rect.x + 10.0 + HEADER_ICON + 8.0;
        let label = self.fitted(
            &title,
            TextStyle::new(MENU_TEXT, palette.ink),
            chevron_x - text_x - 4.0,
        );
        Self::centered_text(rect, text_x, label, &mut row);
        Self::centered_text(rect, chevron_x, chevron, &mut row);
        if self.key_display {
            items.extend(row);
        } else {
            items.push(Item::Opacity {
                alpha: DIMMED,
                items: row,
            });
        }
    }

    fn paint_pill(&mut self, rect: Rect, index: usize, items: &mut Vec<Item>) {
        let palette = self.palette;
        let active = index == self.workspaces.1;
        let (fill, ink) = if active {
            (palette.ink, palette.chassis)
        } else {
            (palette.well, palette.ink)
        };
        items.push(Item::Fill {
            shape: Shape::rounded(rect, 5.0),
            paint: fill.into(),
        });
        let number = self.paragraph(
            &(index + 1).to_string(),
            TextStyle::new(CAPTION_SIZE, ink)
                .weight(TextStyle::SEMIBOLD)
                .tabular(),
        );
        let x = rect.x + (rect.width - number.width()) / 2.0;
        Self::centered_text(rect, x, number, items);
    }

    fn paint_summary(&mut self, rect: Rect, items: &mut Vec<Item>) {
        let network = self
            .status
            .wifi
            .as_ref()
            .and_then(|wifi| wifi.current())
            .map(|network| network.name());
        let battery = self.status.battery.map(|battery| {
            L10N.tr_args(
                "battery-percent",
                [("percent", battery.percent.round().into())],
            )
        });
        let summary = [network, battery]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
        if summary.is_empty() {
            return;
        }
        let text = self.fitted(
            &summary,
            TextStyle::new(CAPTION_SIZE, self.palette.ink2),
            rect.width - 16.0,
        );
        items.push(Item::Text {
            origin: Point::new(
                rect.x + ROW_INSET,
                (rect.y + rect.height - 6.0 - text.height()).round(),
            ),
            paragraph: text,
        });
    }

    fn paint_long_clock(&mut self, rect: Rect, scale: f64, items: &mut Vec<Item>) {
        let palette = self.palette;
        let time = match &self.clock.period {
            Some(period) => format!("{} {period}", self.clock.time),
            None => self.clock.time.clone(),
        };
        let time = self.paragraph(
            &time,
            TextStyle::new(EXPANDED_TIME, palette.ink)
                .weight(TextStyle::SEMIBOLD)
                .tabular(),
        );
        let long = self.clock.long.clone();
        let date = self.paragraph(&long, TextStyle::new(CAPTION_SIZE, palette.ink2));
        let top = rect.y + (rect.height - time.height() - date.height()) / 2.0;
        let x = 2.0 * ROW_INSET + 4.0;
        clock_mark(
            self.clock_mark,
            Rect::new(
                rect.x + rect.width - 2.0 * ROW_INSET - UNREAD_DOT,
                top + 4.0,
                UNREAD_DOT,
                UNREAD_DOT,
            ),
            palette,
            items,
        );
        items.push(Item::Text {
            origin: Point::new(x, snap(top + time.height(), scale)),
            paragraph: date,
        });
        items.push(Item::Text {
            origin: Point::new(x, snap(top, scale)),
            paragraph: time,
        });
    }

    fn caption(&mut self, width: f32) -> Paragraph {
        let style = TextStyle::new(CAPTION_SIZE, self.palette.ink).weight(TextStyle::SEMIBOLD);
        let mut name: Vec<char> = self.app.chars().collect();
        let mut caption = self.app.clone();
        loop {
            let paragraph = self.paragraph(&caption, style);
            if paragraph.width() <= width || name.len() <= 1 {
                return paragraph;
            }
            name.pop();
            caption = format!("{}…", name.iter().collect::<String>().trim_end());
        }
    }

    fn paint_header(&mut self, rect: Rect, scale: f64, items: &mut Vec<Item>) {
        if self.expanded {
            self.paint_wide_header(rect, scale, items);
            return;
        }
        let mut header = vec![Item::Fill {
            shape: Shape::rounded(rect, GROUP_RADIUS),
            paint: self.palette.ink.with_alpha(WELL_ALPHA).into(),
        }];
        if let Some(alpha) = self.slot_alpha(Slot::App) {
            header.push(Item::Fill {
                shape: Shape::rounded(rect, GROUP_RADIUS),
                paint: self.palette.ink.with_alpha(alpha).into(),
            });
        }
        let icon_top = rect.y + 8.0;
        if let Some(icon) = self.app_icon.clone() {
            header.push(Item::Image {
                rect: Rect::new(
                    snap(rect.x + (rect.width - HEADER_ICON) / 2.0, scale),
                    snap(icon_top, scale),
                    HEADER_ICON,
                    HEADER_ICON,
                ),
                image: icon,
            });
        }
        let chevron = self.paragraph(
            "›",
            TextStyle::new(CAPTION_SIZE, self.palette.ink2).weight(TextStyle::SEMIBOLD),
        );
        let caption = self.caption(rect.width - 8.0 - chevron.width() - 2.0);
        let total = caption.width() + 2.0 + chevron.width();
        let x = snap(rect.x + (rect.width - total) / 2.0, scale);
        let y = snap(icon_top + HEADER_ICON + 3.0, scale);
        header.push(Item::Text {
            origin: Point::new(snap(x + caption.width() + 2.0, scale), y),
            paragraph: chevron,
        });
        header.push(Item::Text {
            origin: Point::new(x, y),
            paragraph: caption,
        });
        if self.key_display {
            items.extend(header);
        } else {
            items.push(Item::Opacity {
                alpha: DIMMED,
                items: header,
            });
        }
    }

    fn paint_wide_header(&mut self, rect: Rect, scale: f64, items: &mut Vec<Item>) {
        let palette = self.palette;
        let group = Rect::new(
            rect.x,
            rect.y,
            rect.width,
            rect.height + self.menus.len() as f32 * MENU_ROW,
        );
        let mut header = vec![Item::Fill {
            shape: Shape::rounded(group, GROUP_RADIUS),
            paint: palette.ink.with_alpha(WELL_ALPHA).into(),
        }];
        if let Some(alpha) = self.slot_alpha(Slot::App) {
            header.push(Item::Fill {
                shape: Shape::rounded(rect, GROUP_RADIUS),
                paint: palette.ink.with_alpha(alpha).into(),
            });
        }
        if let Some(icon) = self.app_icon.clone() {
            header.push(Item::Image {
                rect: Rect::new(
                    snap(rect.x + 10.0, scale),
                    snap(rect.y + (rect.height - HEADER_ICON) / 2.0, scale),
                    HEADER_ICON,
                    HEADER_ICON,
                ),
                image: icon,
            });
        }
        let name_x = rect.x + 10.0 + HEADER_ICON + 8.0;
        let app = self.app.clone();
        let name = self.fitted(
            &app,
            TextStyle::new(MENU_TEXT + 1.0, palette.ink).weight(TextStyle::BOLD),
            rect.x + rect.width - name_x - 10.0,
        );
        Self::centered_text(rect, name_x, name, &mut header);
        if self.key_display {
            items.extend(header);
        } else {
            items.push(Item::Opacity {
                alpha: DIMMED,
                items: header,
            });
        }
    }

    fn paint_clock(&mut self, rect: Rect, scale: f64, items: &mut Vec<Item>) {
        let palette = self.palette;
        let time = self.clock.time.clone();
        let day = match &self.clock.period {
            Some(period) => format!("{period} · {}", self.clock.day),
            None => self.clock.day.clone(),
        };
        let time = self.paragraph(
            &time,
            TextStyle::new(TIME_SIZE, palette.ink)
                .weight(TextStyle::SEMIBOLD)
                .tabular(),
        );
        let day = self.paragraph(&day, TextStyle::new(DAY_SIZE, palette.ink2));
        let total = time.height() + day.height();
        let top = rect.y + (rect.height - total) / 2.0;
        let time_x = snap(rect.x + (rect.width - time.width()) / 2.0, scale);
        clock_mark(
            self.clock_mark,
            Rect::new(
                (time_x + time.width() + 1.0).min(rect.x + rect.width - UNREAD_DOT - 1.0),
                top,
                UNREAD_DOT,
                UNREAD_DOT,
            ),
            palette,
            items,
        );
        items.push(Item::Text {
            origin: Point::new(
                snap(rect.x + (rect.width - day.width()) / 2.0, scale),
                snap(top + time.height(), scale),
            ),
            paragraph: day,
        });
        items.push(Item::Text {
            origin: Point::new(time_x, snap(top, scale)),
            paragraph: time,
        });
    }

    fn describe(&self, slot: Slot) -> String {
        match slot {
            Slot::System if self.expanded => L10N.tr("system-menu"),
            Slot::System | Slot::Launcher => L10N.tr("launcher"),
            Slot::App => L10N.tr_args("app-menus", [("app", self.app.clone().into())]),
            Slot::Tray(index) => self
                .tray
                .get(index)
                .map(|(title, _)| title.clone())
                .unwrap_or_default(),
            Slot::TrayMore => L10N.tr("more-status-items"),
            Slot::InputSource => self
                .status
                .input_sources
                .current()
                .map(|source| L10N.tr_args("input-source", [("name", source.name.clone().into())]))
                .unwrap_or_default(),
            Slot::Bluetooth => L10N.tr("bluetooth"),
            Slot::Sound => L10N.tr("sound"),
            Slot::Wifi => L10N.tr("wifi"),
            Slot::Battery => self.status.battery.map_or_else(String::new, |battery| {
                L10N.tr_args(
                    "battery-level",
                    [("percent", battery.percent.round().into())],
                )
            }),
            Slot::ControlCenter => control_center_label(&self.status),
            Slot::Clock => {
                let clock = format!("{} {}", self.clock.time, self.clock.day);
                match self.clock_mark {
                    ClockMark::Clear => clock,
                    ClockMark::Unread => L10N.tr_args("clock-unread", [("time", clock.into())]),
                    ClockMark::Quiet => L10N.tr_args("clock-quiet", [("time", clock.into())]),
                }
            }
            Slot::Workspace(index) => {
                L10N.tr_args("workspace", [("number", (index as i64 + 1).into())])
            }
            Slot::AppMenu(_) => String::new(),
        }
    }
}

impl StatusView for RailView {
    fn set_app(&mut self, name: &str) -> bool {
        let changed = self.app != name;
        name.clone_into(&mut self.app);
        changed
    }

    fn app(&self) -> &str {
        &self.app
    }

    fn set_app_menus(&mut self, titles: Vec<String>) {
        self.menus = titles;
    }

    fn set_app_icon(&mut self, icon: Option<Image>) {
        self.app_icon = icon;
    }

    fn set_key_display(&mut self, key: bool) -> bool {
        let changed = self.key_display != key;
        self.key_display = key;
        changed
    }

    fn cascades(&self) -> bool {
        true
    }

    fn set_tray(&mut self, items: Vec<(String, Option<Image>)>) {
        self.tray = items;
    }

    fn status(&self) -> &Status {
        &self.status
    }

    fn status_mut(&mut self) -> &mut Status {
        &mut self.status
    }

    fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    fn set_time(&mut self, at: &Zoned, dates: &DateFormats) -> bool {
        let (time, period) = dates.time(at);
        let day = dates.short_day(at);
        let long = dates.long_date(at);
        let changed = self.clock.time != time
            || self.clock.period != period
            || self.clock.day != day
            || self.clock.long != long;
        self.clock = Clock {
            time,
            period,
            day,
            long,
        };
        changed
    }

    fn set_open(&mut self, slot: Option<Slot>) -> bool {
        let changed = self.open != slot;
        self.open = slot;
        changed
    }

    fn open(&self) -> Option<Slot> {
        self.open
    }

    fn slot_at(&self, point: Point, button: PointerButton) -> Option<Slot> {
        let found = self
            .slots
            .iter()
            .filter(|(_, rect)| rect.contains(point))
            .map(|(slot, _)| *slot)
            .collect::<Vec<_>>();
        let glyph = found.iter().copied().find(|slot| {
            matches!(
                slot,
                Slot::Wifi | Slot::Sound | Slot::Bluetooth | Slot::Battery
            )
        });
        match (button, glyph, found.first()) {
            (PointerButton::Secondary, Some(glyph), _) => Some(glyph),
            (_, Some(_), _) => Some(Slot::ControlCenter),
            (PointerButton::Secondary, None, found) => found.copied(),
            (_, None, Some(Slot::System)) if !self.expanded => Some(Slot::Launcher),
            (_, None, found) => found.copied(),
        }
    }

    fn slot_rect(&self, slot: Slot) -> Option<Rect> {
        self.slots
            .iter()
            .find(|(found, _)| *found == slot)
            .map(|(_, rect)| *rect)
    }

    fn slot_of_node(&self, node: NodeId, action: Action) -> Option<Slot> {
        let index = usize::try_from(node.0.checked_sub(NODE_BASE)?).ok()?;
        self.slots.get(index).map(|(slot, _)| match slot {
            Slot::System if action == Action::Click && !self.expanded => Slot::Launcher,
            slot => *slot,
        })
    }

    fn edge(&self) -> DisplayEdge {
        DisplayEdge::Left
    }

    fn menu_placement(&self, anchor: Rect) -> MenuPlacement {
        MenuPlacement::AtPoint(Point::new(self.width + GROUP_GAP, anchor.y))
    }

    fn slides_between_menus(&self) -> bool {
        false
    }

    fn set_clock_mark(&mut self, mark: ClockMark) -> bool {
        let changed = self.clock_mark != mark;
        self.clock_mark = mark;
        changed
    }

    fn set_battery_percentage(&mut self, shown: bool) -> bool {
        let changed = self.battery_percentage != shown;
        self.battery_percentage = shown;
        changed
    }

    fn battery_percentage(&self) -> bool {
        self.battery_percentage
    }

    fn set_workspaces(&mut self, count: usize, active: usize) -> bool {
        let changed = self.workspaces != (count, active);
        self.workspaces = (count, active);
        changed
    }

    fn hidden_tray(&self) -> Vec<Slot> {
        self.tray_split().1
    }

    fn set_hovered(&mut self, slot: Option<Slot>) -> bool {
        let changed = self.hovered != slot;
        self.hovered = slot;
        changed
    }

    fn scene(&mut self, width: u32, height: u32, scale: f64) -> Scene {
        let items = self.paint(self.width.min(width as f32), height as f32, scale);
        Scene {
            width,
            height,
            background: Color::TRANSPARENT,
            items,
        }
    }

    fn accessibility(&self) -> TreeUpdate {
        let root_id = NodeId(NODE_BASE - 1);
        let mut root = AccessNode::new(Role::Group);
        root.set_label(L10N.tr("status-area"));
        let mut nodes = Vec::new();
        for (index, (slot, rect)) in self.slots.iter().enumerate() {
            if matches!(
                slot,
                Slot::Wifi | Slot::Sound | Slot::Bluetooth | Slot::Battery
            ) {
                continue;
            }
            let mut node = AccessNode::new(Role::Button);
            node.set_label(self.describe(*slot));
            if *slot == Slot::ControlCenter {
                let glyphs = [Slot::Wifi, Slot::Sound, Slot::Bluetooth, Slot::Battery]
                    .into_iter()
                    .filter(|glyph| self.slots.iter().any(|(known, _)| known == glyph))
                    .map(|glyph| self.describe(glyph))
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>();
                node.set_description(glyphs.join(", "));
            }
            node.set_bounds(accesskit::Rect {
                x0: f64::from(rect.x),
                y0: f64::from(rect.y),
                x1: f64::from(rect.x + rect.width),
                y1: f64::from(rect.y + rect.height),
            });
            node.add_action(Action::Click);
            if *slot == Slot::System && !self.expanded {
                node.add_action(Action::ShowContextMenu);
            }
            nodes.push((NodeId(NODE_BASE + index as u64), node));
        }
        root.set_children(nodes.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        nodes.insert(0, (root_id, root));
        TreeUpdate {
            nodes,
            tree: Some(TreeInfo::new(root_id)),
            tree_id: TreeId::ROOT,
            focus: root_id,
        }
    }
}

fn clock_mark(mark: ClockMark, dot: Rect, palette: Palette, items: &mut Vec<Item>) {
    let disc = |rect: Rect, paint: Color| Item::Fill {
        shape: Shape::rounded(rect, rect.width / 2.0),
        paint: paint.into(),
    };
    match mark {
        ClockMark::Clear => {}
        ClockMark::Unread => items.push(disc(dot, palette.accent)),
        ClockMark::Quiet => {
            let bite = Rect::new(dot.x + 2.5, dot.y - 1.5, dot.width, dot.height);
            items.push(disc(dot, palette.ink3));
            items.push(disc(bite, palette.chassis));
        }
    }
}
