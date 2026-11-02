use std::collections::HashMap;
use std::path::PathBuf;

use accesskit::{Action, Node as AccessNode, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
use bande_config::BarEdge;
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
use crate::menu_bar::{Slot, StatusView};
use crate::taskbar::{TaskbarLayout as AppsRegion, WELL_ALPHA, vertical};

pub const EDGE_PADDING: f32 = 24.0;
pub const LABEL_ROOM: f32 = 280.0;
const MARK_CELL: f32 = 52.0;
const MARK_SIZE: f32 = 22.0;
const CLOCK_CELL: f32 = 52.0;
const CLOCK_LENGTH: f32 = 80.0;
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

pub fn bar_width(icon_size: f32) -> f32 {
    icon_size + EDGE_PADDING
}

pub fn surface_size(edge: BarEdge, icon_size: f32) -> (u32, u32) {
    let depth = (bar_width(icon_size) + LABEL_ROOM).ceil() as u32;
    match vertical(edge) {
        true => (depth, 0),
        false => (0, depth),
    }
}

pub fn edge_band(edge: BarEdge, size: (f32, f32), depth: f32) -> Rect {
    let (width, height) = size;
    match edge {
        BarEdge::Left => Rect::new(0.0, 0.0, depth, height),
        BarEdge::Right => Rect::new(width - depth, 0.0, depth, height),
        BarEdge::Top => Rect::new(0.0, 0.0, width, depth),
        BarEdge::Bottom => Rect::new(0.0, height - depth, width, depth),
    }
}

pub fn display_edge(edge: BarEdge) -> DisplayEdge {
    match edge {
        BarEdge::Left => DisplayEdge::Left,
        BarEdge::Right => DisplayEdge::Right,
        BarEdge::Top => DisplayEdge::Top,
        BarEdge::Bottom => DisplayEdge::Bottom,
    }
}

pub fn owns_node(node: NodeId) -> bool {
    node.0 >= NODE_BASE - 1
}

struct Clock {
    time: String,
    period: Option<String>,
    day: String,
}

pub struct TaskbarView {
    palette: Palette,
    text: TextSystem,
    edge: BarEdge,
    bar: Rect,
    icon_dirs: Vec<PathBuf>,
    icons: HashMap<(String, Color, u32), Option<Image>>,
    width: f32,
    icon_size: f32,
    app: String,
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

impl TaskbarView {
    pub fn new(
        palette: Palette,
        text: TextSystem,
        edge: BarEdge,
        icon_dirs: Vec<PathBuf>,
        icon_size: f32,
    ) -> Self {
        Self {
            palette,
            text,
            edge,
            bar: Rect::default(),
            icon_dirs,
            icons: HashMap::new(),
            width: bar_width(icon_size),
            icon_size,
            app: String::new(),
            tray: Vec::new(),
            status: Status::default(),
            clock: Clock {
                time: String::new(),
                period: None,
                day: String::new(),
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

    pub fn edge(&self) -> BarEdge {
        self.edge
    }

    pub fn unread(&self) -> bool {
        self.clock_mark == ClockMark::Unread
    }

    pub fn set_icon_size(&mut self, icon_size: f32) {
        self.icon_size = icon_size;
        self.width = bar_width(icon_size);
    }

    fn tray_split(&self) -> (Vec<Slot>, Vec<Slot>) {
        let mut cells: Vec<Slot> = (0..self.tray.len()).map(Slot::StatusItem).collect();
        if self.status.input_sources.current().is_some() {
            cells.push(Slot::InputSource);
        }
        let hidden = cells.split_off(TRAY_SHOWN.min(cells.len()));
        (cells, hidden)
    }

    fn tray_cells(&self) -> Vec<Slot> {
        let (mut cells, hidden) = self.tray_split();
        if !hidden.is_empty() {
            cells.push(Slot::MoreStatusItems);
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

    fn layout(&self, size: (f32, f32)) -> (Vec<(Slot, Rect)>, AppsRegion, Vec<Rect>) {
        let thick = self.width;
        let bar = edge_band(self.edge, size, thick);
        let vertical = vertical(self.edge);
        let (origin, length) = match vertical {
            true => (bar.y, bar.height),
            false => (bar.x, bar.width),
        };
        let place = |along: f32, extent: f32, cross: f32| {
            let offset = ((thick - cross) / 2.0).round();
            match vertical {
                true => Rect::new(bar.x + offset, along, cross, extent),
                false => Rect::new(along, bar.y + offset, extent, cross),
            }
        };
        let mut slots = vec![(Slot::BandeMenu, place(origin, MARK_CELL, thick))];
        let clock = if vertical { CLOCK_CELL } else { CLOCK_LENGTH };
        let mut end = origin + length - clock;
        slots.push((Slot::Clock, place(end, clock, thick)));
        let (status_along, status_cross) = match vertical {
            true => (STATUS_HEIGHT, STATUS_WIDTH),
            false => (STATUS_WIDTH, STATUS_HEIGHT),
        };
        end -= status_along + GROUP_GAP;
        let status = place(end, status_along, status_cross);
        slots.push((Slot::ControlCenter, status));
        let (column_width, row_height) = (status.width / 2.0, status.height / 2.0);
        for (index, (slot, _)) in self.glyph_slots().into_iter().enumerate() {
            let column = (index % 2) as f32;
            let row = (index / 2) as f32;
            slots.push((
                slot,
                Rect::new(
                    status.x + column * column_width,
                    status.y + row * row_height,
                    column_width,
                    row_height,
                ),
            ));
        }
        let tray = self.tray_cells();
        if !tray.is_empty() {
            let lines = tray.len().div_ceil(2) as f32;
            end -= GROUP_GAP + lines * GRID_CELL;
            let group = place(end, lines * GRID_CELL, 2.0 * GRID_CELL);
            for (index, slot) in tray.into_iter().enumerate() {
                let across = (index % 2) as f32;
                let along = (index / 2) as f32;
                let (column, row) = if vertical {
                    (across, along)
                } else {
                    (along, across)
                };
                slots.push((
                    slot,
                    Rect::new(
                        group.x + column * GRID_CELL,
                        group.y + row * GRID_CELL,
                        GRID_CELL,
                        GRID_CELL,
                    ),
                ));
            }
        }
        let (count, active) = self.workspaces;
        if count >= 2 {
            end -= GROUP_GAP + count as f32 * WORKSPACE_CELL + (WORKSPACE_PILL - WORKSPACE_DOT);
            let mut at = end;
            for index in 0..count {
                let extent = if index == active {
                    WORKSPACE_PILL
                } else {
                    WORKSPACE_DOT
                } + (WORKSPACE_CELL - WORKSPACE_DOT);
                slots.push((Slot::Workspace(index), place(at, extent, thick)));
                at += extent;
            }
        }
        end -= HAIRLINE_CELL;
        let hairline = |center: f32| place((center - 0.5).round(), 1.0, HAIRLINE);
        let start = origin + MARK_CELL + HAIRLINE_CELL;
        let hairlines = vec![
            hairline(end + HAIRLINE_CELL / 2.0),
            hairline(origin + MARK_CELL + HAIRLINE_CELL / 2.0),
        ];
        let region = AppsRegion {
            start,
            end: end.max(start),
            bar,
        };
        (slots, region, hairlines)
    }

    pub fn apps_region(&self, size: (f32, f32)) -> AppsRegion {
        self.layout(size).1
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
        let (inset_x, inset_y) = match vertical(self.edge) {
            true => (6.0, 4.0),
            false => (4.0, 6.0),
        };
        let inset = match slot {
            Slot::BandeMenu | Slot::Clock => Rect::new(
                rect.x + inset_x,
                rect.y + inset_y,
                rect.width - 2.0 * inset_x,
                rect.height - 2.0 * inset_y,
            ),
            Slot::Workspace(_) => return,
            _ => rect,
        };
        items.push(Item::Fill {
            shape: Shape::rounded(inset, GROUP_RADIUS),
            paint: self.palette.ink.with_alpha(alpha).into(),
        });
    }

    fn paint(&mut self, size: (f32, f32), scale: f64) -> Vec<Item> {
        let (slots, region, hairlines) = self.layout(size);
        let bar = region.bar;
        self.bar = bar;
        let palette = self.palette;
        let ink = palette.ink;
        let inner_edge = match self.edge {
            BarEdge::Left => Rect::new(bar.x + bar.width - 1.0, bar.y, 1.0, bar.height),
            BarEdge::Right => Rect::new(bar.x, bar.y, 1.0, bar.height),
            BarEdge::Top => Rect::new(bar.x, bar.y + bar.height - 1.0, bar.width, 1.0),
            BarEdge::Bottom => Rect::new(bar.x, bar.y, bar.width, 1.0),
        };
        let mut items = vec![
            Item::Fill {
                shape: Shape::rect(bar),
                paint: palette.chassis.into(),
            },
            Item::Fill {
                shape: Shape::rect(inner_edge),
                paint: palette.edge.into(),
            },
        ];
        for hairline in hairlines {
            items.push(Item::Fill {
                shape: Shape::rect(hairline),
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
                Slot::BandeMenu => {
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
                Slot::StatusItem(index) => {
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
                Slot::MoreStatusItems => {
                    let text = self.paragraph("•••", TextStyle::new(BADGE_SIZE, ink));
                    items.push(Item::Text {
                        origin: Point::new(
                            snap(rect.x + (rect.width - text.width()) / 2.0, scale),
                            snap(rect.y + (rect.height - text.height()) / 2.0, scale),
                        ),
                        paragraph: text,
                    });
                }
                Slot::Workspace(index) => {
                    let active = index == self.workspaces.1;
                    let length = if active {
                        WORKSPACE_PILL
                    } else {
                        WORKSPACE_DOT
                    };
                    let (dot_width, dot_height) = match vertical(self.edge) {
                        true => (WORKSPACE_DOT, length),
                        false => (length, WORKSPACE_DOT),
                    };
                    let dot = Rect::new(
                        (rect.x + (rect.width - dot_width) / 2.0).round(),
                        (rect.y + (rect.height - dot_height) / 2.0).round(),
                        dot_width,
                        dot_height,
                    );
                    items.push(Item::Fill {
                        shape: Shape::rounded(dot, WORKSPACE_DOT / 2.0),
                        paint: if active { ink } else { palette.ink2 }.into(),
                    });
                }
                Slot::Clock => self.paint_clock(rect, scale, &mut items),
                _ => {}
            }
            if slot == Slot::ControlCenter && !self.status.capturing.is_empty() {
                items.push(in_use_dot(rect, &palette));
            }
        }
        self.slots = slots;
        items
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
            Slot::BandeMenu | Slot::Search => L10N.tr("launcher"),
            Slot::StatusItem(index) => self
                .tray
                .get(index)
                .map(|(title, _)| title.clone())
                .unwrap_or_default(),
            Slot::MoreStatusItems => L10N.tr("more-status-items"),
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
            Slot::AppMenu | Slot::AppMenuTitle(_) => String::new(),
        }
    }
}

impl StatusView for TaskbarView {
    fn set_app(&mut self, name: &str) -> bool {
        let changed = self.app != name;
        name.clone_into(&mut self.app);
        changed
    }

    fn app(&self) -> &str {
        &self.app
    }

    fn set_app_menus(&mut self, _titles: Vec<String>) {}

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
        let changed =
            self.clock.time != time || self.clock.period != period || self.clock.day != day;
        self.clock = Clock { time, period, day };
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
            (_, None, Some(Slot::BandeMenu)) => Some(Slot::Search),
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
            Slot::BandeMenu if action == Action::Click => Slot::Search,
            slot => *slot,
        })
    }

    fn edge(&self) -> DisplayEdge {
        display_edge(self.edge)
    }

    fn menu_placement(&self, anchor: Rect) -> MenuPlacement {
        let bar = self.bar;
        MenuPlacement::AtPoint(match self.edge {
            BarEdge::Left => Point::new(bar.x + bar.width + GROUP_GAP, anchor.y),
            BarEdge::Right => Point::new(bar.x - GROUP_GAP, anchor.y),
            BarEdge::Top => Point::new(anchor.x, bar.y + bar.height + GROUP_GAP),
            BarEdge::Bottom => Point::new(anchor.x, bar.y - GROUP_GAP),
        })
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
        let items = self.paint((width as f32, height as f32), scale);
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
            if *slot == Slot::BandeMenu {
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
