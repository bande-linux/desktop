use std::cell::RefCell;
use std::collections::HashMap;
use std::f32::consts::TAU;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Node, Role};
use unbound_config::DockEdge;
use unbound_l10n::Localizer;
use unbound_toolkit::ui::{
    Animated, Context, Event, PointerButton, Size, Spring, Ui, Widget, WidgetId,
};
use unbound_toolkit::{
    Appearance, Color, Image, Item, Paint, Palette, Paragraph, Point, Rect, Scene, Scheme, Shape,
    Stop, TextAlign, TextStyle, TextSystem,
};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-dock.ftl");

pub mod autostart;
pub mod bus;

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub const ICON_SIZE: f32 = 48.0;
const MIN_ICON_SIZE: f32 = 24.0;
const MAX_ICON_SIZE: f32 = 96.0;
const CELL_PADDING: f32 = 5.0;
const CROSS_PADDING: f32 = 8.0;
const END_PADDING: f32 = 6.0;
const SCREEN_GAP: f32 = 6.0;
const SEPARATOR_CELL: f32 = 13.0;
const LABEL_ROOM: f32 = 280.0;
const LABEL_GAP: f32 = 10.0;
const LABEL_TEXT: f32 = 13.0;
const LABEL_DELAY: Duration = Duration::from_millis(400);
const LABEL_GRACE: Duration = Duration::from_millis(250);
const DOT: f32 = 4.0;
const PRESSED_SCALE: f32 = 0.86;
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(10);
const LAUNCH_PERIOD: f32 = 1.2;
const ATTENTION_PULSES: f32 = 2.0;
const ATTENTION_PERIOD: f32 = 0.6;
const BADGE_HEIGHT: f32 = 18.0;
const BADGE_TEXT: f32 = 11.0;
const PROGRESS_HEIGHT: f32 = 4.0;
const HOLD_FOR_MENU: Duration = Duration::from_millis(500);
const MENU_GAP: f32 = 6.0;
const REVEAL_DWELL: Duration = Duration::from_millis(120);
const HIDE_DELAY: Duration = Duration::from_millis(450);
const EDGE_STRIP: f32 = 2.0;
const HIDDEN_SHADOW: f32 = 24.0;
const BLUR_BANDS: usize = 5;

#[derive(Debug, Clone)]
pub struct DockApp {
    pub app_id: String,
    pub name: String,
    pub icon: Option<Image>,
    pub kept: bool,
    pub running: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LauncherUpdate {
    pub count: Option<i64>,
    pub count_visible: Option<bool>,
    pub progress: Option<f64>,
    pub progress_visible: Option<bool>,
    pub urgent: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct LauncherEntry {
    count: i64,
    count_visible: bool,
    progress: f64,
    progress_visible: bool,
    urgent: bool,
}

impl LauncherEntry {
    fn merge(&mut self, update: LauncherUpdate) {
        self.count = update.count.unwrap_or(self.count);
        self.count_visible = update.count_visible.unwrap_or(self.count_visible);
        self.progress = update.progress.unwrap_or(self.progress);
        self.progress_visible = update.progress_visible.unwrap_or(self.progress_visible);
        self.urgent = update.urgent.unwrap_or(self.urgent);
    }

    fn badge(&self) -> Option<i64> {
        (self.count_visible && self.count > 0).then_some(self.count)
    }

    fn progress(&self) -> Option<f64> {
        self.progress_visible
            .then_some(self.progress.clamp(0.0, 1.0))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuTarget {
    App(String),
    Trash,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DockAction {
    Open(String),
    OpenTrash,
    ShowInFiles(String),
    Menu { target: MenuTarget, at: Point },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Key {
    App(String),
    RunningSeparator,
    TrashSeparator,
    Trash,
}

impl Key {
    fn is_separator(&self) -> bool {
        matches!(self, Self::RunningSeparator | Self::TrashSeparator)
    }
}

struct Slot {
    key: Key,
    id: WidgetId,
    app: Option<DockApp>,
    presence: Animated,
    leaving: bool,
    dot: Animated,
    press: Animated,
    badge: Animated,
    progress: Animated,
    pressed: bool,
    initial: Option<Paragraph>,
    badge_text: Option<(String, Paragraph)>,
    cell: Rect,
}

impl Slot {
    fn new(key: Key, presence: f32) -> Self {
        Self {
            key,
            id: WidgetId::next(),
            app: None,
            presence: Animated::new(presence),
            leaving: false,
            dot: Animated::new(0.0),
            press: Animated::new(1.0),
            badge: Animated::new(0.0),
            progress: Animated::new(0.0),
            pressed: false,
            initial: None,
            badge_text: None,
            cell: Rect::default(),
        }
    }

    fn cell_length(&self, icon: f32) -> f32 {
        if self.key.is_separator() {
            SEPARATOR_CELL
        } else {
            icon + 2.0 * CELL_PADDING
        }
    }
}

#[derive(Default)]
struct TrashIcons {
    empty: Option<Image>,
    full: Option<Image>,
    is_full: bool,
}

struct Label {
    key: Option<Key>,
    pending: Option<(Key, Instant)>,
    opacity: Animated,
    along: Animated,
    hidden_at: Option<Instant>,
    text: Option<(String, Color, Paragraph)>,
}

pub struct DockState {
    edge: DockEdge,
    icon_size: f32,
    scheme: Scheme,
    reduce_motion: bool,
    slots: Vec<Slot>,
    trash: TrashIcons,
    launching: HashMap<String, Instant>,
    attention: HashMap<String, Instant>,
    launcher: HashMap<String, LauncherEntry>,
    label: Label,
    actions: Vec<DockAction>,
    hold: Option<(Key, Instant)>,
    surface: Size,
    panel: Rect,
    populated: bool,
    blurred: bool,
    auto_hide: bool,
    reveal: Animated,
    reveal_at: Option<Instant>,
    hide_at: Option<Instant>,
    pointer_inside: bool,
    menu_open: bool,
}

pub type Shared = Rc<RefCell<DockState>>;

fn vertical(edge: DockEdge) -> bool {
    matches!(edge, DockEdge::Left | DockEdge::Right)
}

pub fn thickness(icon_size: f32) -> f32 {
    icon_size + 2.0 * CROSS_PADDING
}

pub fn exclusive_zone(icon_size: f32) -> f32 {
    thickness(icon_size) + 2.0 * SCREEN_GAP
}

pub fn surface_depth(icon_size: f32) -> f32 {
    exclusive_zone(icon_size) + LABEL_ROOM
}

fn contains(frame: Rect, point: Point) -> bool {
    point.x >= frame.x
        && point.y >= frame.y
        && point.x < frame.x + frame.width
        && point.y < frame.y + frame.height
}

fn scaled(rect: Rect, factor: f32) -> Rect {
    let width = rect.width * factor;
    let height = rect.height * factor;
    Rect::new(
        rect.x + (rect.width - width) / 2.0,
        rect.y + (rect.height - height) / 2.0,
        width,
        height,
    )
}

fn badge_label(count: i64) -> String {
    if count > 99 {
        "99+".to_owned()
    } else {
        count.to_string()
    }
}

impl DockState {
    pub fn new(edge: DockEdge, icon_size: f32) -> Shared {
        let mut state = Self {
            edge,
            icon_size,
            scheme: Scheme::Light,
            reduce_motion: false,
            slots: Vec::new(),
            trash: TrashIcons::default(),
            launching: HashMap::new(),
            attention: HashMap::new(),
            launcher: HashMap::new(),
            label: Label {
                key: None,
                pending: None,
                opacity: Animated::new(0.0),
                along: Animated::new(0.0),
                hidden_at: None,
                text: None,
            },
            actions: Vec::new(),
            hold: None,
            surface: Size::default(),
            panel: Rect::default(),
            populated: false,
            blurred: false,
            auto_hide: false,
            reveal: Animated::new(1.0),
            reveal_at: None,
            hide_at: None,
            pointer_inside: false,
            menu_open: false,
        };
        state.slots.push(Slot::new(Key::RunningSeparator, 0.0));
        state.slots.push(Slot::new(Key::TrashSeparator, 1.0));
        state.slots.push(Slot::new(Key::Trash, 1.0));
        Rc::new(RefCell::new(state))
    }

    pub fn edge(&self) -> DockEdge {
        self.edge
    }

    pub fn icon_size(&self) -> f32 {
        self.icon_size
    }

    pub fn set_icon_size(&mut self, size: f32) -> bool {
        let size = size.clamp(MIN_ICON_SIZE, MAX_ICON_SIZE).round();
        if size == self.icon_size {
            return false;
        }
        self.icon_size = size;
        for slot in &mut self.slots {
            slot.initial = None;
        }
        true
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.scheme = appearance.scheme;
        self.reduce_motion = appearance.reduce_motion;
    }

    pub fn set_trash_icons(&mut self, empty: Option<Image>, full: Option<Image>) {
        self.trash.empty = empty;
        self.trash.full = full;
    }

    pub fn set_trash_full(&mut self, full: bool) {
        self.trash.is_full = full;
    }

    pub fn set_apps(&mut self, apps: Vec<DockApp>, now: Instant) {
        let animate = self.populated;
        self.populated = true;
        let reduce_motion = self.reduce_motion;
        let stopped = self
            .slots
            .iter()
            .filter_map(|slot| slot.app.as_ref())
            .filter(|known| known.running)
            .filter(|known| {
                !apps
                    .iter()
                    .any(|app| app.app_id == known.app_id && app.running)
            })
            .map(|known| known.app_id.clone())
            .collect::<Vec<_>>();
        for app_id in stopped {
            self.launcher.remove(&app_id);
            self.attention.remove(&app_id);
        }
        for app in &apps {
            if app.running {
                self.launching.remove(&app.app_id);
            }
        }
        let order = self.slots.iter().map(|slot| slot.id).collect::<Vec<_>>();
        let mut old = std::mem::take(&mut self.slots);
        let mut take = |key: &Key| {
            old.iter()
                .position(|slot| &slot.key == key && !slot.leaving)
                .map(|index| old.remove(index))
        };
        let mut slots = Vec::new();
        let any_running = apps.iter().any(|app| !app.kept);
        let mut separator_placed = false;
        for app in apps {
            if !app.kept && !separator_placed {
                separator_placed = true;
                slots.extend(take(&Key::RunningSeparator));
            }
            let key = Key::App(app.app_id.clone());
            let mut slot =
                take(&key).unwrap_or_else(|| Slot::new(key, if animate { 0.0 } else { 1.0 }));
            let running = app.running || self.launching.contains_key(&app.app_id);
            slot.dot.animate_at(
                now,
                reduce_motion,
                if running { 1.0 } else { 0.0 },
                Spring::FADE,
            );
            if !animate {
                slot.dot.jump_to(if running { 1.0 } else { 0.0 });
            }
            slot.presence
                .animate_at(now, reduce_motion, 1.0, Spring::STANDARD);
            if slot
                .app
                .as_ref()
                .is_some_and(|known| known.name != app.name)
            {
                slot.initial = None;
            }
            slot.app = Some(app);
            slots.push(slot);
        }
        if !separator_placed {
            slots.extend(take(&Key::RunningSeparator));
        }
        for key in [Key::TrashSeparator, Key::Trash] {
            slots.extend(take(&key));
        }
        for slot in &mut slots {
            if slot.key == Key::RunningSeparator {
                let target = if any_running { 1.0 } else { 0.0 };
                slot.presence
                    .animate_at(now, reduce_motion, target, Spring::STANDARD);
                if !animate {
                    slot.presence.jump_to(target);
                }
            }
        }
        for mut slot in old {
            if !animate {
                continue;
            }
            slot.leaving = true;
            slot.presence
                .animate_at(now, reduce_motion, 0.0, Spring::STANDARD);
            slot.dot.animate_at(now, reduce_motion, 0.0, Spring::FADE);
            let at = order
                .iter()
                .take_while(|id| **id != slot.id)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .find_map(|id| slots.iter().position(|known| known.id == *id))
                .map_or(0, |index| index + 1);
            slots.insert(at, slot);
        }
        self.slots = slots;
        if let Some(Key::App(app_id)) = &self.label.key
            && !self
                .slots
                .iter()
                .any(|slot| slot.key == Key::App(app_id.clone()) && !slot.leaving)
        {
            self.hide_label(now);
        }
    }

    pub fn launching(&mut self, app_id: &str, now: Instant) {
        self.launching.insert(app_id.to_owned(), now);
        let reduce_motion = self.reduce_motion;
        if let Some(slot) = self.slot_mut(&Key::App(app_id.to_owned())) {
            slot.dot.animate_at(now, reduce_motion, 1.0, Spring::FADE);
        }
    }

    pub fn is_launching(&self, app_id: &str) -> bool {
        self.launching.contains_key(app_id)
    }

    pub fn request_attention(&mut self, app_id: &str, now: Instant) {
        self.attention.insert(app_id.to_owned(), now);
    }

    pub fn activated(&mut self, app_id: &str) {
        self.attention.remove(app_id);
        if let Some(state) = self.launcher.get_mut(app_id) {
            state.urgent = false;
        }
    }

    pub fn update_launcher(&mut self, app_id: &str, update: LauncherUpdate, now: Instant) {
        let before = self.launcher.get(app_id).copied().unwrap_or_default();
        let mut entry = before;
        entry.merge(update);
        if entry.urgent && !before.urgent {
            self.attention.insert(app_id.to_owned(), now);
        }
        if !entry.urgent && before.urgent {
            self.attention.remove(app_id);
        }
        self.launcher.insert(app_id.to_owned(), entry);
        self.refresh_launcher(app_id, now);
    }

    fn refresh_launcher(&mut self, app_id: &str, now: Instant) {
        let entry = self.launcher.get(app_id).copied().unwrap_or_default();
        let reduce_motion = self.reduce_motion;
        if let Some(slot) = self.slot_mut(&Key::App(app_id.to_owned())) {
            let shown = if entry.badge().is_some() { 1.0 } else { 0.0 };
            slot.badge
                .animate_at(now, reduce_motion, shown, Spring::BOUNCY);
            if let Some(progress) = entry.progress() {
                slot.progress
                    .animate_at(now, reduce_motion, progress as f32, Spring::STANDARD);
            }
        }
    }

    pub fn take_actions(&mut self) -> Vec<DockAction> {
        std::mem::take(&mut self.actions)
    }

    pub fn deadline(&self) -> Option<Instant> {
        let label = self
            .label
            .pending
            .as_ref()
            .map(|(_, since)| *since + LABEL_DELAY);
        let launch = self
            .launching
            .values()
            .map(|since| *since + LAUNCH_TIMEOUT)
            .min();
        let hold = self.hold.as_ref().map(|(_, since)| *since + HOLD_FOR_MENU);
        [label, launch, hold, self.reveal_at, self.hide_at]
            .into_iter()
            .flatten()
            .min()
    }

    pub fn tick(&mut self, now: Instant) {
        if self.reveal_at.is_some_and(|at| now >= at) {
            self.reveal_at = None;
            self.slide(true, now);
        }
        if self.hide_at.is_some_and(|at| now >= at) {
            self.hide_at = None;
            self.hide_label(now);
            self.slide(false, now);
        }
        if let Some((key, since)) = self.hold.clone()
            && now >= since + HOLD_FOR_MENU
        {
            self.hold = None;
            let reduce_motion = self.reduce_motion;
            if let Some(slot) = self.slot_mut(&key) {
                slot.pressed = false;
                slot.press
                    .animate_at(now, reduce_motion, 1.0, Spring::QUICK);
            }
            self.open_menu(&key);
        }
        if let Some((key, since)) = self.label.pending.clone()
            && now >= since + LABEL_DELAY
        {
            self.show_label(key, now);
        }
        let expired = self
            .launching
            .iter()
            .filter(|(_, since)| now >= **since + LAUNCH_TIMEOUT)
            .map(|(app_id, _)| app_id.clone())
            .collect::<Vec<_>>();
        let reduce_motion = self.reduce_motion;
        for app_id in expired {
            self.launching.remove(&app_id);
            if let Some(slot) = self.slot_mut(&Key::App(app_id))
                && !slot.app.as_ref().is_some_and(|app| app.running)
            {
                slot.dot.animate_at(now, reduce_motion, 0.0, Spring::FADE);
            }
        }
    }

    pub fn pulsing(&self, now: Instant) -> bool {
        !self.reduce_motion
            && (!self.launching.is_empty()
                || self.attention.values().any(|since| {
                    now.saturating_duration_since(*since).as_secs_f32()
                        < ATTENTION_PULSES * ATTENTION_PERIOD
                }))
    }

    pub fn panel(&self) -> Rect {
        self.panel
    }

    pub fn set_auto_hide(&mut self, auto_hide: bool, now: Instant) {
        self.auto_hide = auto_hide;
        self.reveal_at = None;
        self.hide_at = None;
        let shown = !auto_hide || self.pointer_inside || self.menu_open;
        self.slide(shown, now);
    }

    pub fn set_menu_open(&mut self, open: bool, now: Instant) {
        self.menu_open = open;
        if !open && self.auto_hide && !self.pointer_inside {
            self.hide_at = Some(now + HIDE_DELAY);
        }
    }

    pub fn is_shown(&self) -> bool {
        self.reveal.target() > 0.0
    }

    pub fn exclusive_zone(&self) -> i32 {
        if self.auto_hide {
            0
        } else {
            exclusive_zone(self.icon_size).round() as i32
        }
    }

    pub fn input_region(&self) -> Rect {
        let panel = self.panel;
        let size = self.surface;
        let depth = if self.is_shown() {
            SCREEN_GAP + panel.width.min(panel.height)
        } else {
            EDGE_STRIP
        };
        let along = if vertical(self.edge) {
            (panel.y, panel.height)
        } else {
            (panel.x, panel.width)
        };
        match self.edge {
            DockEdge::Left => Rect::new(0.0, along.0, depth, along.1),
            DockEdge::Right => Rect::new(size.width - depth, along.0, depth, along.1),
            DockEdge::Top => Rect::new(along.0, 0.0, along.1, depth),
            DockEdge::Bottom => Rect::new(along.0, size.height - depth, along.1, depth),
        }
    }

    fn slide(&mut self, shown: bool, now: Instant) {
        let target = if shown { 1.0 } else { 0.0 };
        self.reveal
            .animate_at(now, self.reduce_motion, target, Spring::STANDARD);
    }

    fn pointer_entered(&mut self, now: Instant) {
        self.pointer_inside = true;
        self.hide_at = None;
        if self.auto_hide && !self.is_shown() && self.reveal_at.is_none() {
            self.reveal_at = Some(now + REVEAL_DWELL);
        }
    }

    fn pointer_left(&mut self, now: Instant) {
        self.pointer_inside = false;
        self.reveal_at = None;
        if self.auto_hide && !self.menu_open && self.is_shown() {
            self.hide_at = Some(now + HIDE_DELAY);
        }
    }

    pub fn icon_rects(&self) -> Vec<(String, Rect)> {
        self.slots
            .iter()
            .filter(|slot| !slot.leaving)
            .filter_map(|slot| match &slot.key {
                Key::App(app_id) => Some((app_id.clone(), self.icon_rect(slot.cell))),
                _ => None,
            })
            .collect()
    }

    fn slot_mut(&mut self, key: &Key) -> Option<&mut Slot> {
        self.slots
            .iter_mut()
            .find(|slot| &slot.key == key && !slot.leaving)
    }

    fn effective_icon(&self, length: f32) -> f32 {
        let (cells, separators) =
            self.slots
                .iter()
                .fold((0.0, 0.0), |(cells, separators), slot| {
                    let presence = slot.presence.target();
                    if slot.key.is_separator() {
                        (cells, separators + presence * SEPARATOR_CELL)
                    } else {
                        (cells + presence, separators)
                    }
                });
        if cells <= 0.0 {
            return self.icon_size;
        }
        let room = length - 2.0 * SCREEN_GAP - 2.0 * END_PADDING - separators;
        let fitting = room / cells - 2.0 * CELL_PADDING;
        self.icon_size.min(fitting).max(16.0)
    }

    fn icon_rect(&self, cell: Rect) -> Rect {
        let icon = self.effective_icon(self.along(self.surface));
        Rect::new(
            cell.x + (cell.width - icon) / 2.0,
            cell.y + (cell.height - icon) / 2.0,
            icon,
            icon,
        )
    }

    fn along(&self, size: Size) -> f32 {
        if vertical(self.edge) {
            size.height
        } else {
            size.width
        }
    }

    fn layout(&mut self, size: Size, now: Instant) {
        self.surface = size;
        let along_length = self.along(size);
        let icon = self.effective_icon(along_length);
        let thick = thickness(self.icon_size);
        let lengths = self
            .slots
            .iter()
            .map(|slot| slot.cell_length(icon) * slot.presence.value_at(now).0.max(0.0))
            .collect::<Vec<_>>();
        let total = 2.0 * END_PADDING + lengths.iter().sum::<f32>();
        let start = ((along_length - total) / 2.0).max(SCREEN_GAP);
        let edge = self.edge;
        let hidden = (1.0 - self.reveal.value_at(now).0) * (SCREEN_GAP + thick + HIDDEN_SHADOW);
        let cross = match edge {
            DockEdge::Left | DockEdge::Top => SCREEN_GAP - hidden,
            DockEdge::Right => size.width - SCREEN_GAP - thick + hidden,
            DockEdge::Bottom => size.height - SCREEN_GAP - thick + hidden,
        };
        let place = |along: f32, length: f32| {
            if vertical(edge) {
                Rect::new(cross, along, thick, length)
            } else {
                Rect::new(along, cross, length, thick)
            }
        };
        self.panel = place(start, total);
        let mut offset = start + END_PADDING;
        for (slot, length) in self.slots.iter_mut().zip(lengths) {
            let full = slot.cell_length(icon);
            slot.cell = place(offset - (full - length) / 2.0, full);
            offset += length;
        }
        self.slots
            .retain(|slot| !(slot.leaving && !slot.presence.value_at(now).1));
    }

    fn reach(&self, cell: Rect) -> Rect {
        match self.edge {
            DockEdge::Left => Rect::new(0.0, cell.y, cell.x + cell.width, cell.height),
            DockEdge::Right => Rect::new(cell.x, cell.y, self.surface.width - cell.x, cell.height),
            DockEdge::Top => Rect::new(cell.x, 0.0, cell.width, cell.y + cell.height),
            DockEdge::Bottom => Rect::new(cell.x, cell.y, cell.width, self.surface.height - cell.y),
        }
    }

    fn hover_enter(&mut self, key: Key, now: Instant) {
        let recently_shown = self.label.opacity.target() > 0.0
            || self
                .label
                .hidden_at
                .is_some_and(|hidden| now.saturating_duration_since(hidden) < LABEL_GRACE);
        if recently_shown {
            self.show_label(key, now);
        } else {
            self.label.pending = Some((key, now));
        }
    }

    fn hover_leave(&mut self, key: &Key, now: Instant) {
        if self
            .label
            .pending
            .as_ref()
            .is_some_and(|(pending, _)| pending == key)
        {
            self.label.pending = None;
        }
        if self.label.key.as_ref() == Some(key) {
            self.hide_label(now);
        }
    }

    fn show_label(&mut self, key: Key, now: Instant) {
        self.label.pending = None;
        let Some(slot) = self.slots.iter().find(|slot| slot.key == key) else {
            return;
        };
        let center = if vertical(self.edge) {
            slot.cell.y + slot.cell.height / 2.0
        } else {
            slot.cell.x + slot.cell.width / 2.0
        };
        let first = self.label.opacity.value_at(now).0 <= 0.01;
        if first {
            self.label.along.jump_to(center);
        } else {
            self.label
                .along
                .animate_at(now, self.reduce_motion, center, Spring::QUICK);
        }
        self.label
            .opacity
            .animate_at(now, self.reduce_motion, 1.0, Spring::FADE);
        self.label.key = Some(key);
        self.label.hidden_at = None;
    }

    fn hide_label(&mut self, now: Instant) {
        self.label.pending = None;
        if self.label.opacity.target() > 0.0 {
            self.label.hidden_at = Some(now);
        }
        self.label
            .opacity
            .animate_at(now, self.reduce_motion, 0.0, Spring::FADE);
    }

    fn name(&self, key: &Key) -> String {
        match key {
            Key::App(app_id) => self
                .slots
                .iter()
                .find(|slot| &slot.key == key)
                .and_then(|slot| slot.app.as_ref())
                .map_or_else(|| app_id.clone(), |app| app.name.clone()),
            _ => L10N.tr("trash"),
        }
    }

    fn menu_point(&self, key: &Key) -> Option<Point> {
        let slot = self.slots.iter().find(|slot| &slot.key == key)?;
        let icon = self.icon_rect(slot.cell);
        let panel = self.panel;
        Some(match self.edge {
            DockEdge::Left => Point::new(panel.x + panel.width + MENU_GAP, icon.y),
            DockEdge::Right => Point::new(panel.x - MENU_GAP, icon.y),
            DockEdge::Top => Point::new(icon.x, panel.y + panel.height + MENU_GAP),
            DockEdge::Bottom => Point::new(icon.x, panel.y - MENU_GAP),
        })
    }

    fn open_menu(&mut self, key: &Key) {
        let target = match key {
            Key::App(app_id) => MenuTarget::App(app_id.clone()),
            Key::Trash => MenuTarget::Trash,
            Key::RunningSeparator | Key::TrashSeparator => return,
        };
        let Some(at) = self.menu_point(key) else {
            return;
        };
        self.label.pending = None;
        self.label.opacity.jump_to(0.0);
        self.actions.push(DockAction::Menu { target, at });
    }

    fn show_in_files(&mut self, key: &Key) {
        if let Key::App(app_id) = key {
            self.actions.push(DockAction::ShowInFiles(app_id.clone()));
        }
    }

    fn activate(&mut self, key: &Key) {
        match key {
            Key::App(app_id) => self.actions.push(DockAction::Open(app_id.clone())),
            Key::Trash => self.actions.push(DockAction::OpenTrash),
            Key::RunningSeparator | Key::TrashSeparator => {}
        }
    }

    fn prepare_text(&mut self, cx: &mut Context) {
        for slot in &mut self.slots {
            if let Some(app) = &slot.app
                && app.icon.is_none()
                && slot.initial.is_none()
            {
                let initial = app
                    .name
                    .chars()
                    .next()
                    .map(|character| character.to_uppercase().collect::<String>())
                    .unwrap_or_default();
                slot.initial = Some(
                    cx.text.paragraph(
                        &initial,
                        TextStyle::new((self.icon_size * 0.5).round(), Color::hex(0xFFFFFF))
                            .weight(TextStyle::SEMIBOLD),
                        None,
                        TextAlign::Start,
                    ),
                );
            }
            let count = match &slot.key {
                Key::App(app_id) => self.launcher.get(app_id).and_then(LauncherEntry::badge),
                _ => None,
            };
            if let Some(count) = count.filter(|count| *count > 0) {
                let text = badge_label(count);
                if slot
                    .badge_text
                    .as_ref()
                    .is_none_or(|(known, _)| known != &text)
                {
                    let paragraph = cx.text.paragraph(
                        &text,
                        TextStyle::new(BADGE_TEXT, Color::hex(0xFFFFFF))
                            .weight(TextStyle::SEMIBOLD),
                        None,
                        TextAlign::Start,
                    );
                    slot.badge_text = Some((text, paragraph));
                }
            }
        }
        if let Some(key) = self.label.key.clone() {
            let name = self.name(&key);
            if self.label.text.as_ref().is_none_or(|(known, color, _)| {
                known != &name || *color != cx.palette.label_primary
            }) {
                let color = cx.palette.label_primary;
                let paragraph = cx.text.paragraph(
                    &name,
                    TextStyle::new(LABEL_TEXT, color).weight(TextStyle::MEDIUM),
                    None,
                    TextAlign::Start,
                );
                self.label.text = Some((name, color, paragraph));
            }
        }
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let now = cx.now();
        self.paint_panel(items);
        for slot in &self.slots {
            let presence = slot.presence.value(cx).clamp(0.0, 1.0);
            if presence <= 0.0 {
                continue;
            }
            let mut slot_items = Vec::new();
            match &slot.key {
                Key::RunningSeparator | Key::TrashSeparator => {
                    self.paint_separator(cx, slot, &mut slot_items);
                }
                Key::Trash => {
                    let rect = scaled(self.icon_rect(slot.cell), slot.press.value(cx));
                    let image = if self.trash.is_full {
                        self.trash.full.as_ref().or(self.trash.empty.as_ref())
                    } else {
                        self.trash.empty.as_ref()
                    };
                    match image {
                        Some(image) => slot_items.push(Item::Image {
                            rect,
                            image: image.clone(),
                        }),
                        None => generic_icon(rect, None, &mut slot_items),
                    }
                }
                Key::App(app_id) => self.paint_app(cx, slot, app_id, now, &mut slot_items),
            }
            if presence < 1.0 {
                items.push(Item::Opacity {
                    alpha: presence,
                    items: slot_items,
                });
            } else {
                items.extend(slot_items);
            }
        }
        self.paint_label(cx, items);
    }

    fn panel_radius(&self) -> f32 {
        (thickness(self.icon_size) * 0.3).round()
    }

    pub fn set_blurred(&mut self, blurred: bool) {
        self.blurred = blurred;
    }

    pub fn blur_rects(&self) -> Vec<Rect> {
        let panel = self.panel;
        let radius = self
            .panel_radius()
            .min(panel.width / 2.0)
            .min(panel.height / 2.0);
        if panel.width <= 0.0 || panel.height <= 0.0 {
            return Vec::new();
        }
        let band = radius / BLUR_BANDS as f32;
        let mut rects = vec![Rect::new(
            panel.x,
            panel.y + radius,
            panel.width,
            panel.height - 2.0 * radius,
        )];
        for index in 0..BLUR_BANDS {
            let from_edge = index as f32 * band;
            let rise = radius - from_edge;
            let inset = (radius - (radius * radius - rise * rise).max(0.0).sqrt()).ceil();
            let width = panel.width - 2.0 * inset;
            rects.push(Rect::new(panel.x + inset, panel.y + from_edge, width, band));
            rects.push(Rect::new(
                panel.x + inset,
                panel.y + panel.height - from_edge - band,
                width,
                band,
            ));
        }
        rects
    }

    fn paint_panel(&self, items: &mut Vec<Item>) {
        let dark = self.scheme == Scheme::Dark;
        let radius = self.panel_radius();
        let shape = Shape::rounded(self.panel, radius);
        items.push(Item::Shadow {
            shape,
            offset: Point::new(0.0, 3.0),
            blur_radius: 16.0,
            color: Color::hex(0x000000).with_alpha(if dark { 110 } else { 46 }),
        });
        let (fill, stroke) = match (dark, self.blurred) {
            (true, true) => (
                Color::hex(0x2A2D33).with_alpha(178),
                Color::hex(0xFFFFFF).with_alpha(30),
            ),
            (true, false) => (
                Color::hex(0x2A2D33).with_alpha(232),
                Color::hex(0xFFFFFF).with_alpha(26),
            ),
            (false, true) => (
                Color::hex(0xF7F8FA).with_alpha(170),
                Color::hex(0x000000).with_alpha(26),
            ),
            (false, false) => (
                Color::hex(0xF7F8FA).with_alpha(236),
                Color::hex(0x000000).with_alpha(22),
            ),
        };
        items.push(Item::Fill {
            shape,
            paint: fill.into(),
        });
        items.push(Item::Stroke {
            shape: Shape::rounded(self.panel.inset(0.5), radius - 0.5),
            width: 1.0,
            paint: stroke.into(),
        });
    }

    fn paint_separator(&self, cx: &Context, slot: &Slot, items: &mut Vec<Item>) {
        let cell = slot.cell;
        let span = thickness(self.icon_size) * 0.56;
        let rect = if vertical(self.edge) {
            Rect::new(
                cell.x + (cell.width - span) / 2.0,
                (cell.y + cell.height / 2.0 - 0.5).round(),
                span,
                1.0,
            )
        } else {
            Rect::new(
                (cell.x + cell.width / 2.0 - 0.5).round(),
                cell.y + (cell.height - span) / 2.0,
                1.0,
                span,
            )
        };
        items.push(Item::Fill {
            shape: Shape::rounded(rect, 0.5),
            paint: cx.palette.label_muted.with_alpha(90).into(),
        });
    }

    fn paint_app(
        &self,
        cx: &Context,
        slot: &Slot,
        app_id: &str,
        now: Instant,
        items: &mut Vec<Item>,
    ) {
        let Some(app) = &slot.app else {
            return;
        };
        let base = self.icon_rect(slot.cell);
        let mut factor = slot.press.value(cx);
        let mut alpha = 1.0;
        if !self.reduce_motion {
            if let Some(since) = self.launching.get(app_id) {
                let phase = now.saturating_duration_since(*since).as_secs_f32() / LAUNCH_PERIOD;
                let wave = 0.5 - 0.5 * (phase * TAU).cos();
                alpha -= 0.45 * wave;
                factor *= 1.0 - 0.06 * wave;
            }
            if let Some(since) = self.attention.get(app_id) {
                let elapsed = now.saturating_duration_since(*since).as_secs_f32();
                if elapsed < ATTENTION_PULSES * ATTENTION_PERIOD {
                    let wave = (elapsed / ATTENTION_PERIOD * std::f32::consts::PI)
                        .sin()
                        .abs();
                    factor *= 1.0 + 0.14 * wave;
                }
            }
        }
        let rect = scaled(base, factor);
        let mut icon_items = Vec::new();
        match &app.icon {
            Some(image) => icon_items.push(Item::Image {
                rect,
                image: image.clone(),
            }),
            None => generic_icon(rect, slot.initial.as_ref(), &mut icon_items),
        }
        if slot.pressed {
            alpha *= 0.8;
        }
        if alpha < 1.0 {
            items.push(Item::Opacity {
                alpha,
                items: icon_items,
            });
        } else {
            items.extend(icon_items);
        }
        if self.attention.contains_key(app_id) {
            let ring = rect.inset(-3.0);
            items.push(Item::Stroke {
                shape: Shape::rounded(ring, ring.width * 0.26),
                width: 2.0,
                paint: Color::hex(0xFF9F0A).into(),
            });
        }
        let dot = slot.dot.value(cx);
        if dot > 0.0 {
            let center = self.dot_center(slot.cell, base);
            let size = DOT * (0.5 + 0.5 * dot);
            items.push(Item::Opacity {
                alpha: dot.min(1.0),
                items: vec![Item::Fill {
                    shape: Shape::rounded(
                        Rect::new(center.x - size / 2.0, center.y - size / 2.0, size, size),
                        size / 2.0,
                    ),
                    paint: cx.palette.label_primary.with_alpha(220).into(),
                }],
            });
        }
        let entry = self.launcher.get(app_id).copied().unwrap_or_default();
        if entry.progress().is_some() {
            let progress = slot.progress.value(cx).clamp(0.0, 1.0);
            let track = Rect::new(
                base.x + base.width * 0.12,
                base.y + base.height - PROGRESS_HEIGHT - base.height * 0.08,
                base.width * 0.76,
                PROGRESS_HEIGHT,
            );
            items.push(Item::Fill {
                shape: Shape::rounded(track, PROGRESS_HEIGHT / 2.0),
                paint: Color::hex(0x000000).with_alpha(110).into(),
            });
            let fill = Rect::new(track.x, track.y, track.width * progress, track.height);
            items.push(Item::Fill {
                shape: Shape::rounded(fill, PROGRESS_HEIGHT / 2.0),
                paint: cx.palette.accent_color.into(),
            });
        }
        let badge = slot.badge.value(cx).max(0.0);
        if badge > 0.0
            && let Some((_, text)) = &slot.badge_text
        {
            let width = (text.width() + 10.0).max(BADGE_HEIGHT);
            let rest = Rect::new(
                base.x + base.width - width + 4.0,
                base.y - 4.0,
                width,
                BADGE_HEIGHT,
            );
            let pill = scaled(rest, badge);
            let mut badge_items = vec![
                Item::Shadow {
                    shape: Shape::rounded(pill, pill.height / 2.0),
                    offset: Point::new(0.0, 1.0),
                    blur_radius: 3.0,
                    color: Color::hex(0x000000).with_alpha(60),
                },
                Item::Fill {
                    shape: Shape::rounded(pill, pill.height / 2.0),
                    paint: Color::hex(0xFF3B30).into(),
                },
            ];
            if badge > 0.6 {
                badge_items.push(Item::Text {
                    origin: Point::new(
                        (rest.x + (rest.width - text.width()) / 2.0).round(),
                        (rest.y + (rest.height - text.height()) / 2.0).round(),
                    ),
                    paragraph: text.clone(),
                });
            }
            items.push(Item::Opacity {
                alpha: badge.min(1.0),
                items: badge_items,
            });
        }
    }

    fn dot_center(&self, cell: Rect, icon: Rect) -> Point {
        let inset = CROSS_PADDING / 2.0;
        match self.edge {
            DockEdge::Left => Point::new(cell.x + inset, icon.y + icon.height / 2.0),
            DockEdge::Right => Point::new(cell.x + cell.width - inset, icon.y + icon.height / 2.0),
            DockEdge::Top => Point::new(icon.x + icon.width / 2.0, cell.y + inset),
            DockEdge::Bottom => Point::new(icon.x + icon.width / 2.0, cell.y + cell.height - inset),
        }
    }

    fn paint_label(&self, cx: &Context, items: &mut Vec<Item>) {
        let opacity = self.label.opacity.value(cx).clamp(0.0, 1.0);
        if opacity <= 0.0 {
            return;
        }
        let Some((_, _, text)) = &self.label.text else {
            return;
        };
        let along = self.label.along.value(cx);
        let width = (text.width() + 20.0).ceil();
        let height = (text.height() + 10.0).ceil();
        let panel = self.panel;
        let rect = match self.edge {
            DockEdge::Left => Rect::new(
                panel.x + panel.width + LABEL_GAP,
                along - height / 2.0,
                width,
                height,
            ),
            DockEdge::Right => Rect::new(
                panel.x - LABEL_GAP - width,
                along - height / 2.0,
                width,
                height,
            ),
            DockEdge::Top => Rect::new(
                along - width / 2.0,
                panel.y + panel.height + LABEL_GAP,
                width,
                height,
            ),
            DockEdge::Bottom => Rect::new(
                along - width / 2.0,
                panel.y - LABEL_GAP - height,
                width,
                height,
            ),
        };
        let rect = Rect::new(rect.x.round(), rect.y.round(), rect.width, rect.height);
        let shape = Shape::rounded(rect, height / 2.0);
        let dark = self.scheme == Scheme::Dark;
        let fill = if dark {
            Color::hex(0x2A2D33).with_alpha(245)
        } else {
            Color::hex(0xFFFFFF).with_alpha(245)
        };
        items.push(Item::Opacity {
            alpha: opacity,
            items: vec![
                Item::Shadow {
                    shape,
                    offset: Point::new(0.0, 2.0),
                    blur_radius: 8.0,
                    color: Color::hex(0x000000).with_alpha(if dark { 100 } else { 40 }),
                },
                Item::Fill {
                    shape,
                    paint: fill.into(),
                },
                Item::Stroke {
                    shape: Shape::rounded(rect.inset(0.5), height / 2.0 - 0.5),
                    width: 1.0,
                    paint: if dark {
                        Color::hex(0xFFFFFF).with_alpha(26).into()
                    } else {
                        Color::hex(0x000000).with_alpha(20).into()
                    },
                },
                Item::Text {
                    origin: Point::new(
                        rect.x + 10.0,
                        (rect.y + (height - text.height()) / 2.0).round(),
                    ),
                    paragraph: text.clone(),
                },
            ],
        });
    }
}

fn generic_icon(rect: Rect, initial: Option<&Paragraph>, items: &mut Vec<Item>) {
    let shape = Shape::rounded(rect, rect.width * 0.224);
    items.push(Item::Fill {
        shape,
        paint: Paint::LinearGradient {
            start: Point::new(rect.x, rect.y),
            end: Point::new(rect.x, rect.y + rect.height),
            stops: vec![
                Stop {
                    offset: 0.0,
                    color: Color::hex(0x8E9AAF),
                },
                Stop {
                    offset: 1.0,
                    color: Color::hex(0x5C677D),
                },
            ],
        },
    });
    if let Some(initial) = initial {
        items.push(Item::Text {
            origin: Point::new(
                (rect.x + (rect.width - initial.width()) / 2.0).round(),
                (rect.y + (rect.height - initial.height()) / 2.0).round(),
            ),
            paragraph: initial.clone(),
        });
    }
}

struct Tile {
    id: WidgetId,
    key: Key,
    state: Shared,
    frame: Rect,
}

impl Tile {
    fn press(&self, cx: &Context, pressed: bool, shown: bool) {
        let mut state = self.state.borrow_mut();
        if let Some(slot) = state.slots.iter_mut().find(|slot| slot.key == self.key) {
            slot.pressed = pressed;
            slot.press
                .animate_to(cx, if shown { PRESSED_SCALE } else { 1.0 }, Spring::QUICK);
        }
    }

    fn pressed(&self) -> bool {
        self.state
            .borrow()
            .slots
            .iter()
            .any(|slot| slot.key == self.key && slot.pressed)
    }
}

impl Widget for Tile {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, _available: Size) -> Size {
        Size::new(self.frame.width, self.frame.height)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, _cx: &Context, _items: &mut Vec<Item>) {}

    fn access(&self, _cx: &Context) -> Node {
        let state = self.state.borrow();
        let mut node = Node::new(Role::Button);
        node.set_label(state.name(&self.key));
        let mut details = Vec::new();
        if let Key::App(app_id) = &self.key {
            let slot = state.slots.iter().find(|slot| slot.key == self.key);
            if state.launching.contains_key(app_id) {
                details.push(L10N.tr("opening"));
            } else if slot
                .and_then(|slot| slot.app.as_ref())
                .is_some_and(|app| app.running)
            {
                details.push(L10N.tr("running"));
            }
            let launcher = state.launcher.get(app_id).copied().unwrap_or_default();
            if let Some(count) = launcher.badge() {
                details.push(L10N.tr_args("badge", [("count", count.into())]));
            }
            if let Some(progress) = launcher.progress() {
                let percent = (progress * 100.0).round() as i64;
                details.push(L10N.tr_args("progress", [("percent", percent.into())]));
            }
            if state.attention.contains_key(app_id) {
                details.push(L10N.tr("attention"));
            }
        } else if state.trash.is_full {
            details.push(L10N.tr("trash-full"));
        }
        if !details.is_empty() {
            node.set_description(details.join(", "));
        }
        node.add_action(Action::Click);
        node.add_action(Action::ShowContextMenu);
        node
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        let now = cx.now();
        match event {
            Event::PointerEnter(_) => {
                self.state.borrow_mut().hover_enter(self.key.clone(), now);
            }
            Event::PointerLeave => {
                if self.pressed() {
                    self.press(cx, false, false);
                }
                let mut state = self.state.borrow_mut();
                state.hold = None;
                state.hover_leave(&self.key, now);
            }
            Event::PointerDown(pointer)
                if pointer.button == PointerButton::Secondary
                    || (pointer.is_primary() && pointer.modifiers.control) =>
            {
                self.state.borrow_mut().open_menu(&self.key);
            }
            Event::PointerDown(pointer) if pointer.is_primary() => {
                self.press(cx, true, true);
                let mut state = self.state.borrow_mut();
                state.hide_label(now);
                state.hold = Some((self.key.clone(), now));
            }
            Event::PointerMove(pointer) if self.pressed() => {
                let inside = contains(self.frame, pointer.position);
                if !inside {
                    self.state.borrow_mut().hold = None;
                }
                self.press(cx, true, inside);
            }
            Event::PointerUp(pointer) if pointer.is_primary() => {
                let inside = self.pressed() && contains(self.frame, pointer.position);
                self.press(cx, false, false);
                let mut state = self.state.borrow_mut();
                state.hold = None;
                if inside && pointer.modifiers.logo {
                    state.show_in_files(&self.key);
                } else if inside {
                    state.activate(&self.key);
                }
            }
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        match request.action {
            Action::Click => self.state.borrow_mut().activate(&self.key),
            Action::ShowContextMenu => self.state.borrow_mut().open_menu(&self.key),
            _ => return false,
        }
        true
    }
}

struct Dock {
    id: WidgetId,
    state: Shared,
    children: Vec<Box<dyn Widget>>,
    frame: Rect,
}

impl Widget for Dock {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.state.borrow_mut().prepare_text(cx);
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let mut state = self.state.borrow_mut();
        state.layout(Size::new(frame.width, frame.height), cx.now());
        self.children = state
            .slots
            .iter()
            .filter(|slot| !slot.leaving && !slot.key.is_separator())
            .map(|slot| {
                Box::new(Tile {
                    id: slot.id,
                    key: slot.key.clone(),
                    state: self.state.clone(),
                    frame: state.reach(slot.cell),
                }) as Box<dyn Widget>
            })
            .collect();
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let state = self.state.borrow();
        state.reveal.value(cx);
        state.paint(cx, items);
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        let now = cx.now();
        match event {
            Event::PointerEnter(_) => self.state.borrow_mut().pointer_entered(now),
            Event::PointerLeave => self.state.borrow_mut().pointer_left(now),
            _ => return false,
        }
        true
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Toolbar);
        node.set_label(L10N.tr("dock"));
        node
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub fn dock_ui(state: Shared, palette: Palette, text: TextSystem) -> Ui {
    Ui::new(
        Box::new(Dock {
            id: WidgetId::next(),
            state,
            children: Vec::new(),
            frame: Rect::default(),
        }),
        Context::new(text, palette),
    )
}

pub fn surface_size(edge: DockEdge, icon_size: f32, output: (u32, u32)) -> (u32, u32) {
    let depth = surface_depth(icon_size).ceil() as u32;
    if vertical(edge) {
        (depth, output.1)
    } else {
        (output.0, depth)
    }
}

pub fn scene(ui: &mut Ui, size: (u32, u32)) -> Scene {
    let mut scene = ui.scene(size.0, size.1);
    scene.background = Color::hex(0x000000).with_alpha(0);
    scene
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [include_str!("lib.rs"), include_str!("main.rs")];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
