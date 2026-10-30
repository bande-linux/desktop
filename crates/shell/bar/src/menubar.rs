use accesskit::{Action, Node as AccessNode, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
use jiff::Zoned;
use unbound_datetime::DateFormats;
use unbound_toolkit::ui::PointerButton;
use unbound_toolkit::{
    Color, DisplayEdge, Image, Item, MenuPlacement, Palette, Paragraph, Point, Rect, Scene, Shape,
    TextAlign, TextStyle, TextSystem,
};

use unbound_status::status::{Status, speaker_waves};

use crate::L10N;
use crate::symbols::{Symbol, Symbols, battery_items, place_symbol, snap, wifi_symbol};

pub const HEIGHT: u32 = 24;
const EDGE: f32 = 8.0;
const TEXT_SIZE: f32 = 13.0;
const TEXT_PADDING: f32 = 9.0;
const SYMBOL_SLOT: f32 = 30.0;
const TRAY_SLOT: f32 = 28.0;
const TRAY_ICON: f32 = 16.0;
const BATTERY_SLOT: f32 = 38.0;
const HIGHLIGHT_INSET: f32 = 2.0;
const BADGE_TEXT_SIZE: f32 = 10.0;
const BADGE_PADDING: f32 = 4.0;
const BADGE_MIN_WIDTH: f32 = 22.0;
const BADGE_HEIGHT: f32 = 15.0;
const BADGE_MARGIN: f32 = 5.0;

pub fn app_name<'a>(app_id: &'a str, names: &'a [(String, String)]) -> &'a str {
    names
        .iter()
        .find(|(id, _)| id == app_id)
        .map_or(app_id, |(_, name)| name.as_str())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    System,
    App,
    AppMenu(usize),
    Tray(usize),
    InputSource,
    Bluetooth,
    Sound,
    Wifi,
    Battery,
    ControlCenter,
    Clock,
    TrayMore,
    Workspace(usize),
    Launcher,
}

pub trait StatusView {
    fn set_app(&mut self, name: &str) -> bool;
    fn app(&self) -> &str;
    fn set_app_menus(&mut self, titles: Vec<String>);
    fn set_app_icon(&mut self, icon: Option<Image>);
    fn set_key_display(&mut self, key: bool) -> bool;
    fn cascades(&self) -> bool;
    fn set_tray(&mut self, items: Vec<(String, Option<Image>)>);
    fn status(&self) -> &Status;
    fn status_mut(&mut self) -> &mut Status;
    fn set_palette(&mut self, palette: Palette);
    fn set_time(&mut self, at: &Zoned, dates: &DateFormats) -> bool;
    fn set_open(&mut self, slot: Option<Slot>) -> bool;
    fn open(&self) -> Option<Slot>;
    fn slot_at(&self, point: Point, button: PointerButton) -> Option<Slot>;
    fn slot_rect(&self, slot: Slot) -> Option<Rect>;
    fn slot_of_node(&self, node: NodeId, action: Action) -> Option<Slot>;
    fn menu_placement(&self, anchor: Rect) -> MenuPlacement;
    fn edge(&self) -> DisplayEdge;
    fn slides_between_menus(&self) -> bool;
    fn set_hovered(&mut self, slot: Option<Slot>) -> bool;
    fn set_unread(&mut self, unread: bool) -> bool;
    fn set_battery_percentage(&mut self, shown: bool) -> bool;
    fn battery_percentage(&self) -> bool;
    fn set_workspaces(&mut self, count: usize, active: usize) -> bool;
    fn shown_tray_items(&self) -> usize;
    fn scene(&mut self, width: u32, height: u32, scale: f64) -> Scene;
    fn accessibility(&self) -> TreeUpdate;
}

impl StatusView for MenuBar {
    fn set_app(&mut self, name: &str) -> bool {
        MenuBar::set_app(self, name)
    }

    fn app(&self) -> &str {
        MenuBar::app(self)
    }

    fn set_app_menus(&mut self, titles: Vec<String>) {
        MenuBar::set_app_menus(self, titles);
    }

    fn set_app_icon(&mut self, _icon: Option<Image>) {}

    fn set_key_display(&mut self, _key: bool) -> bool {
        false
    }

    fn cascades(&self) -> bool {
        false
    }

    fn set_tray(&mut self, items: Vec<(String, Option<Image>)>) {
        MenuBar::set_tray(self, items);
    }

    fn status(&self) -> &Status {
        MenuBar::status(self)
    }

    fn status_mut(&mut self) -> &mut Status {
        MenuBar::status_mut(self)
    }

    fn set_palette(&mut self, palette: Palette) {
        MenuBar::set_palette(self, palette);
    }

    fn set_time(&mut self, at: &Zoned, dates: &DateFormats) -> bool {
        self.set_clock(dates.clock(at))
    }

    fn set_open(&mut self, slot: Option<Slot>) -> bool {
        MenuBar::set_open(self, slot)
    }

    fn open(&self) -> Option<Slot> {
        MenuBar::open(self)
    }

    fn slot_at(&self, point: Point, _button: PointerButton) -> Option<Slot> {
        MenuBar::slot_at(self, point)
    }

    fn slot_rect(&self, slot: Slot) -> Option<Rect> {
        MenuBar::slot_rect(self, slot)
    }

    fn slot_of_node(&self, node: NodeId, _action: Action) -> Option<Slot> {
        MenuBar::slot_of_node(self, node)
    }

    fn menu_placement(&self, anchor: Rect) -> MenuPlacement {
        MenuPlacement::Below(anchor)
    }

    fn edge(&self) -> DisplayEdge {
        DisplayEdge::Top
    }

    fn slides_between_menus(&self) -> bool {
        true
    }

    fn set_hovered(&mut self, _slot: Option<Slot>) -> bool {
        false
    }

    fn set_unread(&mut self, _unread: bool) -> bool {
        false
    }

    fn set_battery_percentage(&mut self, shown: bool) -> bool {
        MenuBar::set_battery_percentage(self, shown)
    }

    fn battery_percentage(&self) -> bool {
        MenuBar::battery_percentage(self)
    }

    fn set_workspaces(&mut self, _count: usize, _active: usize) -> bool {
        false
    }

    fn shown_tray_items(&self) -> usize {
        self.tray.len()
    }

    fn scene(&mut self, width: u32, height: u32, scale: f64) -> Scene {
        MenuBar::scene(self, width, height, scale)
    }

    fn accessibility(&self) -> TreeUpdate {
        MenuBar::accessibility(self)
    }
}

pub struct MenuBar {
    palette: Palette,
    text: TextSystem,
    symbols: Symbols,
    app: String,
    app_menus: Vec<String>,
    tray: Vec<(String, Option<Image>)>,
    status: Status,
    clock: String,
    open: Option<Slot>,
    slots: Vec<(Slot, Rect)>,
    battery_percentage: bool,
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
            battery_percentage: false,
        }
    }

    pub fn set_battery_percentage(&mut self, shown: bool) -> bool {
        let changed = self.battery_percentage != shown;
        self.battery_percentage = shown;
        changed
    }

    pub fn battery_percentage(&self) -> bool {
        self.battery_percentage
    }

    fn percent_label(&mut self) -> Option<Paragraph> {
        let battery = self.status.battery.filter(|_| self.battery_percentage)?;
        let text = L10N.tr_args(
            "battery-percent",
            [("percent", battery.percent.round().into())],
        );
        Some(self.label(&text, false))
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

    pub fn set_tray(&mut self, items: Vec<(String, Option<Image>)>) {
        self.tray = items;
    }

    pub fn status_mut(&mut self) -> &mut Status {
        &mut self.status
    }

    pub fn status(&self) -> &Status {
        &self.status
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

    pub fn slot_of_node(&self, node: NodeId) -> Option<Slot> {
        let index = usize::try_from(node.0.checked_sub(1)?).ok()?;
        self.slots.get(index).map(|(slot, _)| *slot)
    }

    fn describe(&self, slot: Slot) -> String {
        match slot {
            Slot::System => L10N.tr("system-menu"),
            Slot::App => self.app.clone(),
            Slot::AppMenu(index) => self.app_menus.get(index).cloned().unwrap_or_default(),
            Slot::Tray(index) => self
                .tray
                .get(index)
                .map(|(title, _)| title.clone())
                .unwrap_or_default(),
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
            Slot::ControlCenter => L10N.tr("control-center"),
            Slot::Clock => self.clock.clone(),
            Slot::TrayMore | Slot::Workspace(_) | Slot::Launcher => String::new(),
        }
    }

    pub fn accessibility(&self) -> TreeUpdate {
        let root_id = NodeId(0);
        let mut root = AccessNode::new(Role::MenuBar);
        root.set_label(L10N.tr("menu-bar"));
        let mut nodes = Vec::new();
        for (index, (slot, rect)) in self.slots.iter().enumerate() {
            let mut node = AccessNode::new(Role::MenuItem);
            node.set_label(self.describe(*slot));
            node.set_bounds(accesskit::Rect {
                x0: f64::from(rect.x),
                y0: f64::from(rect.y),
                x1: f64::from(rect.x + rect.width),
                y1: f64::from(rect.y + rect.height),
            });
            node.add_action(Action::Click);
            nodes.push((NodeId(index as u64 + 1), node));
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

    pub fn slot_rect(&self, slot: Slot) -> Option<Rect> {
        self.slots
            .iter()
            .find(|(found, _)| *found == slot)
            .map(|(_, rect)| *rect)
    }

    fn label(&mut self, text: &str, bold: bool) -> Paragraph {
        let style = TextStyle::new(TEXT_SIZE, self.palette.ink);
        let style = if bold {
            style.weight(TextStyle::SEMIBOLD)
        } else {
            style
        };
        self.text.paragraph(text, style, None, TextAlign::Start)
    }

    fn input_source_badge(&mut self) -> Option<(Paragraph, f32)> {
        let short = self.status.input_sources.current()?.short.clone();
        let paragraph = self.text.paragraph(
            &short,
            TextStyle::new(BADGE_TEXT_SIZE, self.palette.ink).weight(TextStyle::SEMIBOLD),
            None,
            TextAlign::Start,
        );
        let width = (paragraph.width() + 2.0 * BADGE_PADDING).max(BADGE_MIN_WIDTH);
        Some((paragraph, width))
    }

    fn right_slots(&self, badge_width: Option<f32>, percent_width: f32) -> Vec<(Slot, f32)> {
        let mut slots: Vec<(Slot, f32)> = (0..self.tray.len())
            .map(|index| (Slot::Tray(index), TRAY_SLOT))
            .collect();
        if let Some(width) = badge_width {
            slots.push((Slot::InputSource, width + 2.0 * BADGE_MARGIN));
        }
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
            slots.push((Slot::Battery, BATTERY_SLOT + percent_width));
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
            Slot::Wifi => wifi_symbol(self.status.wifi.as_ref()),
            _ => return None,
        })
    }

    fn place_symbol(&mut self, symbol: Symbol, rect: Rect, scale: f64, items: &mut Vec<Item>) {
        place_symbol(
            &mut self.symbols,
            symbol,
            self.palette.ink,
            rect,
            scale,
            items,
        );
    }

    fn paint_badge(
        &self,
        paragraph: Paragraph,
        width: f32,
        rect: Rect,
        scale: f64,
        items: &mut Vec<Item>,
    ) {
        let x = snap(rect.x + (rect.width - width) / 2.0, scale);
        let y = snap(rect.y + (rect.height - BADGE_HEIGHT) / 2.0, scale);
        let badge = Rect::new(x, y, width, BADGE_HEIGHT);
        items.push(Item::Stroke {
            shape: Shape::rounded(badge.inset(0.5), 4.0),
            width: 1.0,
            paint: self.palette.ink.with_alpha(200).into(),
        });
        items.push(Item::Text {
            origin: Point::new(
                snap(x + (width - paragraph.width()) / 2.0, scale),
                snap(y + (BADGE_HEIGHT - paragraph.height()) / 2.0, scale),
            ),
            paragraph,
        });
    }

    pub fn scene(&mut self, width: u32, height: u32, scale: f64) -> Scene {
        let bar = Rect::new(0.0, 0.0, width as f32, height as f32);
        let mut items = vec![
            Item::Fill {
                shape: Shape::rect(bar),
                paint: self.palette.chassis.into(),
            },
            Item::Fill {
                shape: Shape::rect(Rect::new(0.0, bar.height - 1.0, bar.width, 1.0)),
                paint: self.palette.edge.into(),
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
        let badge = self.input_source_badge();
        let percent = self.percent_label();
        let percent_width = percent.as_ref().map_or(0.0, |paragraph| {
            paragraph.width().ceil() + TEXT_PADDING / 2.0
        });
        for (slot, width) in self
            .right_slots(badge.as_ref().map(|(_, width)| *width), percent_width)
            .into_iter()
            .rev()
        {
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
                paint: self.palette.ink.with_alpha(34).into(),
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
                        if let Some(paragraph) = percent.clone() {
                            let y = snap((content.height - paragraph.height()) / 2.0, scale);
                            items.push(Item::Text {
                                origin: Point::new(snap(content.x + TEXT_PADDING / 2.0, scale), y),
                                paragraph,
                            });
                        }
                        let icon = Rect::new(
                            content.x + percent_width,
                            content.y,
                            content.width - percent_width,
                            content.height,
                        );
                        battery_items(
                            &mut self.symbols,
                            &self.palette,
                            battery,
                            icon,
                            scale,
                            &mut items,
                        );
                    }
                }
                Slot::InputSource => {
                    if let Some((paragraph, width)) = badge.clone() {
                        self.paint_badge(paragraph, width, content, scale, &mut items);
                    }
                }
                Slot::Tray(index) => {
                    if let Some((_, Some(image))) = self.tray.get(index).cloned() {
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
