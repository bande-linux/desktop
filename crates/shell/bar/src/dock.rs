use std::cell::RefCell;
use std::collections::HashMap;
use std::f32::consts::TAU;
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Node, Role};
use unbound_config::{BAR_ICON_SIZE_RANGE, BarEdge, DEFAULT_BAR_ICON_SIZE};
use unbound_dock_model::{LauncherEntry, LauncherUpdate};
use unbound_toolkit::ui::{
    Animated, Context, Cursor, Event, Key as KeyboardKey, Modifiers, PointerButton, Size, Spring,
    Ui, Widget, WidgetId,
};

use crate::L10N;
use unbound_toolkit::{
    Appearance, Color, Image, Item, Palette, Paragraph, Point, Rect, Scene, Scheme, Shape,
    TextAlign, TextStyle, TextSystem,
};

pub const ICON_SIZE: f32 = DEFAULT_BAR_ICON_SIZE as f32;
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
const FULL_SCREEN_DWELL: Duration = Duration::from_millis(300);
const HIDE_DELAY: Duration = Duration::from_millis(450);
const EDGE_STRIP: f32 = 2.0;
const HIDDEN_SHADOW: f32 = 24.0;
const DRAG_THRESHOLD: f32 = 4.0;
const DROP_SCALE: f32 = 1.12;

const REMOVE_TEXT: f32 = 12.0;
const RAIL_PADDING: f32 = 4.0;
const RAIL_SHRINK: f32 = 4.0;
const RAIL_HAIRLINE: f32 = 32.0;
const RAIL_RUNNING: f32 = 3.0;
const RAIL_TICK: (f32, f32) = (3.0, 16.0);
const MORE_TEXT: f32 = 13.0;
const RAIL_WELL_RADIUS: f32 = 10.0;
const ROW_ICON: f32 = 30.0;
const ROW_INSET: f32 = 8.0;
const ROW_TEXT_GAP: f32 = 10.0;
const ROW_TEXT: f32 = 13.0;
const ROW_RADIUS: f32 = 6.0;
pub const WELL_ALPHA: u8 = 14;

#[derive(Debug, Clone)]
pub struct DockApp {
    pub app_id: String,
    pub name: String,
    pub icon: Option<Image>,
    pub kept: bool,
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuTarget {
    App(String),
    Trash,
    More(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rail {
    pub top: f32,
    pub bottom: f32,
    pub width: f32,
    pub expanded: bool,
}

struct More {
    hidden: Vec<String>,
    cell: Rect,
    text: Option<(String, Color, Paragraph)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DockAction {
    Open(String),
    OpenTrash,
    ShowInFiles(String),
    Menu { target: MenuTarget, at: Point },
    Keep(Vec<String>),
    ReleaseKeyboard,
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
    name_text: Option<(String, Color, Paragraph)>,
    badge_text: Option<(String, Paragraph)>,
    cell: Rect,
    shift: (Animated, Animated),
    pinned: Option<Point>,
    hidden: bool,
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
            name_text: None,
            badge_text: None,
            cell: Rect::default(),
            shift: (Animated::points(0.0), Animated::points(0.0)),
            pinned: None,
            hidden: false,
        }
    }

    fn kept(&self) -> bool {
        self.app.as_ref().is_some_and(|app| app.kept)
    }

    fn cell_length(&self, icon: f32, padding: f32) -> f32 {
        if self.key.is_separator() {
            SEPARATOR_CELL
        } else {
            icon + 2.0 * padding
        }
    }
}

#[derive(Default)]
struct TrashIcons {
    empty: Option<Image>,
    full: Option<Image>,
    is_full: bool,
}

struct Drag {
    key: Key,
    grab: Point,
    at: Point,
    origin: usize,
    removing: bool,
    kept_before: Vec<String>,
}

struct Ghost {
    icon: Option<Image>,
    initial: Option<Paragraph>,
    center: Point,
    size: f32,
    fade: Animated,
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
    edge: BarEdge,
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
    press_at: Option<Point>,
    drag: Option<Drag>,
    ghost: Option<Ghost>,
    reflow: bool,
    remove_text: Option<(Color, Paragraph)>,
    surface: Size,
    panel: Rect,
    populated: bool,
    auto_hide: bool,
    full_screen: bool,
    reveal: Animated,
    reveal_at: Option<Instant>,
    hide_at: Option<Instant>,
    pointer_inside: bool,
    menu_open: bool,
    selected: Option<Key>,
    rail: Option<Rail>,
    rail_icon: f32,
    active: Option<String>,
    more: Option<More>,
}

pub type Shared = Rc<RefCell<DockState>>;

fn vertical(edge: BarEdge) -> bool {
    matches!(edge, BarEdge::Left | BarEdge::Right)
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
    pub fn new(edge: BarEdge, icon_size: f32) -> Shared {
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
                along: Animated::points(0.0),
                hidden_at: None,
                text: None,
            },
            actions: Vec::new(),
            hold: None,
            press_at: None,
            drag: None,
            ghost: None,
            reflow: false,
            remove_text: None,
            surface: Size::default(),

            panel: Rect::default(),
            populated: false,
            auto_hide: false,
            full_screen: false,
            reveal: Animated::new(1.0),
            reveal_at: None,
            hide_at: None,
            pointer_inside: false,
            menu_open: false,
            selected: None,
            rail: None,
            rail_icon: icon_size,
            active: None,
            more: None,
        };
        state.slots.push(Slot::new(Key::RunningSeparator, 0.0));
        state.slots.push(Slot::new(Key::TrashSeparator, 1.0));
        state.slots.push(Slot::new(Key::Trash, 1.0));
        Rc::new(RefCell::new(state))
    }

    pub fn edge(&self) -> BarEdge {
        self.edge
    }

    pub fn icon_size(&self) -> f32 {
        self.icon_size
    }

    pub fn set_icon_size(&mut self, size: f32) -> bool {
        let (min, max) = BAR_ICON_SIZE_RANGE;
        let size = size.clamp(min as f32, max as f32).round();
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
        if let Some(drag) = &self.drag {
            let key = drag.key.clone();
            let point = drag.at;
            let removing = drag.removing;
            if let Some(slot) = self.slot_mut(&key)
                && removing
            {
                slot.presence
                    .animate_at(now, reduce_motion, 0.0, Spring::STANDARD);
            }
            self.drag_to(point, now);
        }
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

    pub fn set_rail(&mut self, rail: Rail) {
        self.rail = Some(rail);
    }

    pub fn rail(&self) -> Option<Rail> {
        self.rail
    }

    pub fn activated(&mut self, app_id: &str) {
        self.active = (!app_id.is_empty()).then(|| app_id.to_owned());
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
        self.follow_hiding(now);
    }

    pub fn set_full_screen(&mut self, full_screen: bool, now: Instant) {
        self.full_screen = full_screen;
        self.follow_hiding(now);
    }

    fn follow_hiding(&mut self, now: Instant) {
        self.reveal_at = None;
        self.hide_at = None;
        let shown = !self.hides() || self.pointer_inside || self.menu_open;
        self.slide(shown, now);
    }

    fn hides(&self) -> bool {
        self.auto_hide || self.full_screen
    }

    pub fn revealed(&self, now: Instant) -> f32 {
        self.reveal.value_at(now).0.clamp(0.0, 1.0)
    }

    pub fn is_revealing(&self, now: Instant) -> bool {
        self.reveal.value_at(now).1
    }

    pub fn set_menu_open(&mut self, open: bool, now: Instant) {
        self.menu_open = open;
        if !open && self.hides() && !self.pointer_inside {
            self.hide_at = Some(now + HIDE_DELAY);
        }
    }

    pub fn is_shown(&self) -> bool {
        self.reveal.target() > 0.0
    }

    pub fn exclusive_zone(&self) -> i32 {
        if self.hides() {
            return 0;
        }
        match self.rail {
            Some(rail) => rail.width.round() as i32,
            None => exclusive_zone(self.icon_size).round() as i32,
        }
    }

    pub fn input_region(&self) -> Rect {
        if let Some(rail) = self.rail {
            let width = if self.is_shown() {
                rail.width
            } else {
                EDGE_STRIP
            };
            return Rect::new(0.0, 0.0, width, self.surface.height);
        }
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
            BarEdge::Left => Rect::new(0.0, along.0, depth, along.1),
            BarEdge::Right => Rect::new(size.width - depth, along.0, depth, along.1),
            BarEdge::Top => Rect::new(along.0, 0.0, along.1, depth),
            BarEdge::Bottom => Rect::new(along.0, size.height - depth, along.1, depth),
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
        if self.hides() && !self.is_shown() && self.reveal_at.is_none() {
            let dwell = if self.full_screen {
                FULL_SCREEN_DWELL
            } else {
                REVEAL_DWELL
            };
            self.reveal_at = Some(now + dwell);
        }
    }

    fn pointer_left(&mut self, now: Instant) {
        self.pointer_inside = false;
        self.reveal_at = None;
        if self.hides() && !self.menu_open && self.selected.is_none() && self.is_shown() {
            self.hide_at = Some(now + HIDE_DELAY);
        }
    }

    pub fn drop_target(&self, point: Point) -> Option<MenuTarget> {
        self.slots
            .iter()
            .filter(|slot| !slot.leaving && !slot.hidden)
            .find(|slot| contains(self.reach(slot.cell), point))
            .and_then(|slot| match &slot.key {
                Key::App(app_id) => Some(MenuTarget::App(app_id.clone())),
                Key::Trash => Some(MenuTarget::Trash),
                Key::RunningSeparator | Key::TrashSeparator => None,
            })
    }

    pub fn drag_inside(&mut self, inside: bool, now: Instant) {
        match (inside, self.pointer_inside) {
            (true, false) => self.pointer_entered(now),
            (false, true) => self.pointer_left(now),
            _ => {}
        }
    }

    pub fn hover_drop(&mut self, target: Option<&MenuTarget>, now: Instant) {
        let reduce_motion = self.reduce_motion;
        for slot in &mut self.slots {
            let hovered = match (&slot.key, target) {
                (Key::App(app_id), Some(MenuTarget::App(target))) => app_id == target,
                (Key::Trash, Some(MenuTarget::Trash)) => true,
                _ => false,
            };
            if slot.pressed {
                continue;
            }
            let scale = if hovered { DROP_SCALE } else { 1.0 };
            slot.press
                .animate_at(now, reduce_motion, scale, Spring::QUICK);
        }
    }

    pub fn is_navigating(&self) -> bool {
        self.selected.is_some()
    }

    fn reachable(&self) -> Vec<Key> {
        self.slots
            .iter()
            .filter(|slot| !slot.leaving && !slot.hidden && !slot.key.is_separator())
            .map(|slot| slot.key.clone())
            .collect()
    }

    pub fn start_navigation(&mut self, active: Option<&str>, now: Instant) {
        let reachable = self.reachable();
        let chosen = active
            .map(|app_id| Key::App(app_id.to_owned()))
            .filter(|key| reachable.contains(key))
            .or_else(|| reachable.first().cloned());
        let Some(chosen) = chosen else {
            return;
        };
        self.hide_at = None;
        self.reveal_at = None;
        self.slide(true, now);
        self.select(chosen, now);
    }

    pub fn end_navigation(&mut self, now: Instant) {
        if self.selected.take().is_none() {
            return;
        }
        self.hide_label(now);
        if self.hides() && !self.pointer_inside && !self.menu_open {
            self.hide_at = Some(now + HIDE_DELAY);
        }
    }

    fn select(&mut self, key: Key, now: Instant) {
        self.selected = Some(key.clone());
        self.show_label(key, now);
    }

    pub fn navigate(&mut self, key: KeyboardKey, modifiers: Modifiers, now: Instant) -> bool {
        let Some(selected) = self.selected.clone() else {
            return false;
        };
        let reachable = self.reachable();
        let at = reachable
            .iter()
            .position(|known| *known == selected)
            .unwrap_or(0);
        let (back, forward) = if vertical(self.edge) {
            (KeyboardKey::Up, KeyboardKey::Down)
        } else {
            (KeyboardKey::Left, KeyboardKey::Right)
        };
        match key {
            _ if key == back => {
                if let Some(previous) = at.checked_sub(1).and_then(|at| reachable.get(at)) {
                    self.select(previous.clone(), now);
                }
            }
            _ if key == forward => {
                if let Some(next) = reachable.get(at + 1) {
                    self.select(next.clone(), now);
                }
            }
            KeyboardKey::Home => {
                if let Some(first) = reachable.first() {
                    self.select(first.clone(), now);
                }
            }
            KeyboardKey::End => {
                if let Some(last) = reachable.last() {
                    self.select(last.clone(), now);
                }
            }
            KeyboardKey::Enter | KeyboardKey::Space => {
                self.activate(&selected);
                self.actions.push(DockAction::ReleaseKeyboard);
            }
            KeyboardKey::Menu => self.open_menu(&selected),
            KeyboardKey::Function(10) if modifiers.shift => self.open_menu(&selected),
            KeyboardKey::Escape => self.actions.push(DockAction::ReleaseKeyboard),
            _ => return false,
        }
        true
    }

    pub fn icon_rects(&self) -> Vec<(String, Rect)> {
        self.slots
            .iter()
            .filter(|slot| !slot.leaving && !slot.hidden)
            .filter_map(|slot| match &slot.key {
                Key::App(app_id) => Some((app_id.clone(), self.icon_rect(slot.cell))),
                _ => None,
            })
            .collect()
    }

    pub fn set_edge(&mut self, edge: BarEdge, now: Instant) {
        if edge == self.edge {
            return;
        }
        self.edge = edge;
        self.drag = None;
        self.hide_label(now);
        self.label.opacity.jump_to(0.0);
        for slot in &mut self.slots {
            slot.cell = Rect::default();
            slot.pinned = None;
            slot.shift = (Animated::points(0.0), Animated::points(0.0));
        }
        self.reveal.jump_to(0.0);
        let shown = !self.hides() || self.pointer_inside || self.menu_open;
        self.slide(shown, now);
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    fn icon_center(&self, cell: Rect) -> Point {
        Point::new(cell.x + cell.width / 2.0, cell.y + cell.height / 2.0)
    }

    fn along_of(&self, point: Point) -> f32 {
        if vertical(self.edge) {
            point.y
        } else {
            point.x
        }
    }

    fn depth_of(&self, point: Point) -> f32 {
        match self.edge {
            BarEdge::Left => point.x,
            BarEdge::Right => self.surface.width - point.x,
            BarEdge::Top => point.y,
            BarEdge::Bottom => self.surface.height - point.y,
        }
    }

    fn beyond_dock(&self, point: Point) -> bool {
        let dock = self
            .rail
            .map_or(SCREEN_GAP + thickness(self.icon_size), |rail| rail.width);
        self.depth_of(point) > dock + self.icon_size
    }

    fn press_at(&mut self, key: &Key, point: Point) {
        self.press_at = matches!(key, Key::App(_)).then_some(point);
    }

    fn start_drag(&mut self, key: &Key, point: Point, now: Instant) -> bool {
        let Some(origin_point) = self.press_at else {
            return false;
        };
        let moved = (point.x - origin_point.x).hypot(point.y - origin_point.y);
        if moved < DRAG_THRESHOLD {
            return false;
        }
        let Some(origin) = self
            .slots
            .iter()
            .position(|slot| &slot.key == key && !slot.leaving)
        else {
            return false;
        };
        let center = self.icon_center(self.slots[origin].cell);
        self.press_at = None;
        self.hold = None;
        self.hide_label(now);
        self.slots[origin].pinned = Some(center);
        self.slots[origin].shift = (Animated::points(0.0), Animated::points(0.0));
        self.drag = Some(Drag {
            key: key.clone(),
            grab: Point::new(origin_point.x - center.x, origin_point.y - center.y),
            at: point,
            origin,
            removing: false,
            kept_before: self.kept_ids(),
        });
        self.drag_to(point, now);
        true
    }

    fn drag_to(&mut self, point: Point, now: Instant) {
        let Some(drag) = &self.drag else {
            return;
        };
        let key = drag.key.clone();
        let grab = drag.grab;
        let origin = drag.origin;
        let was_removing = drag.removing;
        let Some(index) = self
            .slots
            .iter()
            .position(|slot| slot.key == key && !slot.leaving)
        else {
            self.drag = None;
            return;
        };
        let kept = self.slots[index].kept();
        let half = self.icon_size / 2.0;
        let center = Point::new(
            (point.x - grab.x).clamp(half, (self.surface.width - half).max(half)),
            (point.y - grab.y).clamp(half, (self.surface.height - half).max(half)),
        );
        self.slots[index].pinned = Some(center);
        let removing = kept && self.beyond_dock(point);
        if let Some(drag) = &mut self.drag {
            drag.at = point;
            drag.removing = removing;
        }
        if removing != was_removing {
            let presence = if removing { 0.0 } else { 1.0 };
            self.slots[index].presence.animate_at(
                now,
                self.reduce_motion,
                presence,
                Spring::STANDARD,
            );
        }
        if removing {
            return;
        }
        let along = self.along_of(center);
        let separator = self
            .slots
            .iter()
            .position(|slot| slot.key == Key::RunningSeparator);
        let separator_along =
            separator.map(|separator| self.along_of(self.icon_center(self.slots[separator].cell)));
        let slot = self.slots.remove(index);
        let target = if !kept && separator_along.is_none_or(|boundary| along > boundary) {
            origin.min(self.slots.len())
        } else {
            let kept_slots = self
                .slots
                .iter()
                .enumerate()
                .filter(|(_, slot)| slot.kept() && !slot.leaving)
                .map(|(position, slot)| (position, self.along_of(self.icon_center(slot.cell))))
                .collect::<Vec<_>>();
            kept_slots
                .iter()
                .find(|(_, other)| along < *other)
                .map(|(position, _)| *position)
                .or_else(|| {
                    self.slots
                        .iter()
                        .position(|slot| slot.key == Key::RunningSeparator)
                })
                .unwrap_or(self.slots.len())
        };
        self.slots.insert(target, slot);
        if target != index {
            self.reflow = true;
        }
    }

    fn drop_drag(&mut self, point: Point, now: Instant) {
        self.drag_to(point, now);
        let Some(drag) = self.drag.take() else {
            return;
        };
        let Some(index) = self
            .slots
            .iter()
            .position(|slot| slot.key == drag.key && !slot.leaving)
        else {
            return;
        };
        if drag.removing {
            let size = self.effective_icon(self.along(self.surface));
            let mut fade = Animated::new(1.0);
            fade.animate_at(now, self.reduce_motion, 0.0, Spring::STANDARD);
            let slot = &mut self.slots[index];
            let center = slot.pinned.take();
            let initial = slot.initial.clone();
            if let Some(app) = &mut slot.app {
                app.kept = false;
                if !app.running {
                    let icon = app.icon.clone();
                    self.ghost = center.map(|center| Ghost {
                        icon,
                        initial,
                        center,
                        size,
                        fade,
                    });
                }
            }
        } else {
            let separator = self
                .slots
                .iter()
                .position(|slot| slot.key == Key::RunningSeparator)
                .unwrap_or(self.slots.len());
            if let Some(app) = &mut self.slots[index].app {
                app.kept = index < separator;
            }
        }
        let after = self.kept_ids();
        if after != drag.kept_before {
            self.actions.push(DockAction::Keep(after));
        }
    }

    fn kept_ids(&self) -> Vec<String> {
        self.slots
            .iter()
            .filter(|slot| slot.kept() && !slot.leaving)
            .filter_map(|slot| slot.app.as_ref().map(|app| app.app_id.clone()))
            .collect()
    }

    fn painted_cell(&self, slot: &Slot, cx: &Context) -> Rect {
        let cell = slot.cell;
        match slot.pinned {
            Some(center) => Rect::new(
                center.x - cell.width / 2.0,
                center.y - cell.height / 2.0,
                cell.width,
                cell.height,
            ),
            None => Rect::new(
                cell.x + slot.shift.0.value(cx),
                cell.y + slot.shift.1.value(cx),
                cell.width,
                cell.height,
            ),
        }
    }

    fn settle_moves(&mut self, before: &[(WidgetId, Rect)], now: Instant) {
        let reflow = std::mem::take(&mut self.reflow);
        let dragged = self.drag.as_ref().map(|drag| drag.key.clone());
        let reduce_motion = self.reduce_motion;
        for slot in &mut self.slots {
            if slot.leaving || dragged.as_ref() == Some(&slot.key) {
                continue;
            }
            let from = match slot.pinned.take() {
                Some(center) => Some(Point::new(
                    center.x - slot.cell.width / 2.0,
                    center.y - slot.cell.height / 2.0,
                )),
                None if reflow => before
                    .iter()
                    .find(|(id, _)| *id == slot.id)
                    .filter(|(_, old)| *old != Rect::default())
                    .map(|(_, old)| {
                        Point::new(
                            old.x + slot.shift.0.value_at(now).0,
                            old.y + slot.shift.1.value_at(now).0,
                        )
                    }),
                None => None,
            };
            let Some(from) = from else {
                continue;
            };
            let delta = (from.x - slot.cell.x, from.y - slot.cell.y);
            if delta.0.abs() < 0.5 && delta.1.abs() < 0.5 {
                continue;
            }
            slot.shift.0.jump_to(delta.0);
            slot.shift.1.jump_to(delta.1);
            slot.shift
                .0
                .animate_at(now, reduce_motion, 0.0, Spring::STANDARD);
            slot.shift
                .1
                .animate_at(now, reduce_motion, 0.0, Spring::STANDARD);
        }
    }

    fn slot_mut(&mut self, key: &Key) -> Option<&mut Slot> {
        self.slots
            .iter_mut()
            .find(|slot| &slot.key == key && !slot.leaving)
    }

    fn effective_icon(&self, length: f32) -> f32 {
        if self.rail.is_some() {
            return self.rail_icon;
        }
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

    fn expanded(&self) -> bool {
        self.rail.is_some_and(|rail| rail.expanded)
    }

    fn icon_rect(&self, cell: Rect) -> Rect {
        let icon = self.effective_icon(self.along(self.surface));
        if self.expanded() {
            return Rect::new(
                cell.x + ROW_INSET + 4.0,
                cell.y + (cell.height - icon) / 2.0,
                icon,
                icon,
            );
        }
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
        if let Some(rail) = self.rail {
            self.layout_rail(rail, size, now);
            return;
        }
        self.surface = size;
        let along_length = self.along(size);
        let icon = self.effective_icon(along_length);
        let thick = thickness(self.icon_size);
        let lengths = self
            .slots
            .iter()
            .map(|slot| {
                slot.cell_length(icon, CELL_PADDING) * slot.presence.value_at(now).0.max(0.0)
            })
            .collect::<Vec<_>>();
        let total = 2.0 * END_PADDING + lengths.iter().sum::<f32>();
        let start = ((along_length - total) / 2.0).max(SCREEN_GAP);
        let edge = self.edge;
        let hidden = (1.0 - self.reveal.value_at(now).0) * (SCREEN_GAP + thick + HIDDEN_SHADOW);
        let cross = match edge {
            BarEdge::Left | BarEdge::Top => SCREEN_GAP - hidden,
            BarEdge::Right => size.width - SCREEN_GAP - thick + hidden,
            BarEdge::Bottom => size.height - SCREEN_GAP - thick + hidden,
        };
        let place = |along: f32, length: f32| {
            if vertical(edge) {
                Rect::new(cross, along, thick, length)
            } else {
                Rect::new(along, cross, length, thick)
            }
        };
        self.panel = place(start, total);
        let before = self
            .slots
            .iter()
            .map(|slot| (slot.id, slot.cell))
            .collect::<Vec<_>>();
        let mut offset = start + END_PADDING;
        for (slot, length) in self.slots.iter_mut().zip(lengths) {
            let full = slot.cell_length(icon, CELL_PADDING);
            slot.cell = place(offset - (full - length) / 2.0, full);
            offset += length;
        }
        self.settle_moves(&before, now);
        self.slots
            .retain(|slot| !(slot.leaving && !slot.presence.value_at(now).1));
        if self
            .ghost
            .as_ref()
            .is_some_and(|ghost| !ghost.fade.value_at(now).1)
        {
            self.ghost = None;
        }
    }

    fn layout_rail(&mut self, rail: Rail, size: Size, now: Instant) {
        self.surface = size;
        let pinned_to_bottom = |key: &Key| matches!(key, Key::Trash | Key::TrashSeparator);
        let wanted = |icon: f32| -> f32 {
            self.slots
                .iter()
                .filter(|slot| !pinned_to_bottom(&slot.key))
                .map(|slot| slot.cell_length(icon, RAIL_PADDING) * slot.presence.target())
                .sum()
        };
        let full = if rail.expanded {
            ROW_ICON
        } else {
            self.icon_size
        };
        let trash_cell = full + 2.0 * RAIL_PADDING;
        let room = rail.bottom - rail.top - trash_cell - SEPARATOR_CELL;
        let icon = if wanted(full) <= room {
            full
        } else {
            full - RAIL_SHRINK
        };
        self.rail_icon = icon;
        let overflowing = wanted(icon) > room;
        let more_cell = icon + 2.0 * RAIL_PADDING;
        let limit = if overflowing { room - more_cell } else { room };
        let thick = if rail.expanded {
            rail.width
        } else {
            thickness(self.icon_size)
        };
        let cross = ((rail.width - thick) / 2.0).round();
        self.panel = Rect::new(0.0, rail.top, rail.width, rail.bottom - rail.top);
        let before = self
            .slots
            .iter()
            .map(|slot| (slot.id, slot.cell))
            .collect::<Vec<_>>();
        let mut offset = rail.top;
        let mut hidden = Vec::new();
        let mut more_at = None;
        for slot in &mut self.slots {
            if pinned_to_bottom(&slot.key) {
                continue;
            }
            let length = slot.cell_length(icon, RAIL_PADDING);
            let shown_length = length * slot.presence.value_at(now).0.max(0.0);
            slot.hidden =
                overflowing && (more_at.is_some() || offset + length > rail.top + limit);
            if slot.hidden {
                more_at.get_or_insert(offset);
                if let Some(app) = slot.app.as_ref().filter(|_| !slot.leaving) {
                    hidden.push(app.app_id.clone());
                }
                continue;
            }
            slot.cell = Rect::new(cross, offset - (length - shown_length) / 2.0, thick, length);
            offset += shown_length;
        }
        self.more = more_at.filter(|_| !hidden.is_empty()).map(|at| More {
            hidden,
            cell: Rect::new(cross, at, thick, more_cell),
            text: self.more.take().and_then(|more| more.text),
        });
        let mut bottom = rail.bottom;
        for key in [Key::Trash, Key::TrashSeparator] {
            if let Some(slot) = self.slots.iter_mut().find(|slot| slot.key == key) {
                let length = slot.cell_length(icon, RAIL_PADDING);
                bottom -= length;
                slot.cell = Rect::new(cross, bottom, thick, length);
                slot.hidden = false;
            }
        }
        self.settle_moves(&before, now);
        self.slots
            .retain(|slot| !(slot.leaving && !slot.presence.value_at(now).1));
        if self
            .ghost
            .as_ref()
            .is_some_and(|ghost| !ghost.fade.value_at(now).1)
        {
            self.ghost = None;
        }
    }

    fn reach(&self, cell: Rect) -> Rect {
        if let Some(rail) = self.rail {
            return Rect::new(0.0, cell.y, rail.width, cell.height);
        }
        match self.edge {
            BarEdge::Left => Rect::new(0.0, cell.y, cell.x + cell.width, cell.height),
            BarEdge::Right => Rect::new(cell.x, cell.y, self.surface.width - cell.x, cell.height),
            BarEdge::Top => Rect::new(cell.x, 0.0, cell.width, cell.y + cell.height),
            BarEdge::Bottom => Rect::new(cell.x, cell.y, cell.width, self.surface.height - cell.y),
        }
    }

    fn hover_enter(&mut self, key: Key, now: Instant) {
        if self.expanded() {
            return;
        }
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
            BarEdge::Left => Point::new(panel.x + panel.width + MENU_GAP, icon.y),
            BarEdge::Right => Point::new(panel.x - MENU_GAP, icon.y),
            BarEdge::Top => Point::new(icon.x, panel.y + panel.height + MENU_GAP),
            BarEdge::Bottom => Point::new(icon.x, panel.y - MENU_GAP),
        })
    }

    pub fn trash_menu_point(&self) -> Option<Point> {
        self.menu_point(&Key::Trash)
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

    fn prepare_more(&mut self, cx: &mut Context) {
        if let Some(more) = &mut self.more {
            let label = L10N.tr_args("more-apps", [("count", (more.hidden.len() as i64).into())]);
            let color = cx.palette.ink;
            if more
                .text
                .as_ref()
                .is_none_or(|(known, known_color, _)| *known != label || *known_color != color)
            {
                let paragraph = cx.text.paragraph(
                    &label,
                    TextStyle::new(MORE_TEXT, color).weight(TextStyle::SEMIBOLD),
                    None,
                    TextAlign::Start,
                );
                more.text = Some((label, color, paragraph));
            }
        }
    }

    fn open_more(&mut self) {
        let Some(more) = &self.more else {
            return;
        };
        let at = Point::new(self.panel.x + self.panel.width + MENU_GAP, more.cell.y);
        self.actions.push(DockAction::Menu {
            target: MenuTarget::More(more.hidden.clone()),
            at,
        });
    }

    fn prepare_text(&mut self, cx: &mut Context) {
        let expanded = self.expanded();
        let room = self.rail.map_or(0.0, |rail| rail.width)
            - 2.0 * ROW_INSET
            - ROW_ICON
            - 3.0 * ROW_TEXT_GAP
            - 24.0;
        for slot in &mut self.slots {
            let name = match (&slot.key, &slot.app) {
                (Key::App(_), Some(app)) => Some(app.name.clone()),
                (Key::Trash, _) => Some(L10N.tr("trash")),
                _ => None,
            };
            let ink = cx.palette.ink;
            if let Some(name) = name.filter(|_| expanded)
                && slot
                    .name_text
                    .as_ref()
                    .is_none_or(|(known, color, _)| *known != name || *color != ink)
            {
                let style = TextStyle::new(ROW_TEXT, ink).weight(TextStyle::MEDIUM);
                let mut characters: Vec<char> = name.chars().collect();
                let mut shown = name.clone();
                let paragraph = loop {
                    let paragraph = cx.text.paragraph(&shown, style, None, TextAlign::Start);
                    if paragraph.width() <= room || characters.len() <= 1 {
                        break paragraph;
                    }
                    characters.pop();
                    shown = format!("{}…", characters.iter().collect::<String>().trim_end());
                };
                slot.name_text = Some((name, ink, paragraph));
            }
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
                        TextStyle::new((self.icon_size * 0.5).round(), cx.palette.ink2)
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
                        TextStyle::new(BADGE_TEXT, cx.palette.on_accent)
                            .weight(TextStyle::SEMIBOLD),
                        None,
                        TextAlign::Start,
                    );
                    slot.badge_text = Some((text, paragraph));
                }
            }
        }
        let inverse = cx.palette.sheet;
        if self.drag.is_some()
            && self
                .remove_text
                .as_ref()
                .is_none_or(|(color, _)| *color != inverse)
        {
            let paragraph = cx.text.paragraph(
                &L10N.tr("remove"),
                TextStyle::new(REMOVE_TEXT, inverse).weight(TextStyle::SEMIBOLD),
                None,
                TextAlign::Start,
            );
            self.remove_text = Some((inverse, paragraph));
        }
        if let Some(key) = self.label.key.clone() {
            let name = self.name(&key);
            if self
                .label
                .text
                .as_ref()
                .is_none_or(|(known, color, _)| known != &name || *color != cx.palette.ink)
            {
                let color = cx.palette.ink;
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
        if self.rail.is_none() {
            self.paint_panel(cx, items);
        }
        let dragged = self.drag.as_ref().map(|drag| &drag.key);
        let mut on_top = Vec::new();
        for slot in &self.slots {
            let cell = self.painted_cell(slot, cx);
            if dragged == Some(&slot.key) {
                if let Key::App(app_id) = &slot.key {
                    self.paint_app(cx, slot, cell, app_id, now, &mut on_top);
                }
                continue;
            }
            let presence = slot.presence.value(cx).clamp(0.0, 1.0);
            if presence <= 0.0 || slot.hidden {
                continue;
            }
            let mut slot_items = Vec::new();
            match &slot.key {
                Key::RunningSeparator | Key::TrashSeparator => {
                    self.paint_separator(cx, cell, &mut slot_items);
                }
                Key::Trash => {
                    self.paint_name(slot, cell, &mut slot_items);
                    let rect = scaled(self.icon_rect(cell), slot.press.value(cx));
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
                        None => generic_icon(cx, rect, None, &mut slot_items),
                    }
                }
                Key::App(app_id) => self.paint_app(cx, slot, cell, app_id, now, &mut slot_items),
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
        self.paint_more(cx, items);
        self.paint_ghost(cx, items);
        self.paint_selection(cx, items);
        items.extend(on_top);
        self.paint_remove_label(cx, items);
        self.paint_label(cx, items);
    }

    fn paint_selection(&self, cx: &Context, items: &mut Vec<Item>) {
        let Some(slot) = self
            .selected
            .as_ref()
            .and_then(|key| self.slots.iter().find(|slot| &slot.key == key))
        else {
            return;
        };
        let ring = self.icon_rect(self.painted_cell(slot, cx)).inset(-4.0);
        items.push(Item::Stroke {
            shape: Shape::rounded(ring, ring.width * 0.27),
            width: 3.0,
            paint: cx.palette.accent.with_alpha(200).into(),
        });
    }

    fn paint_ghost(&self, cx: &Context, items: &mut Vec<Item>) {
        let Some(ghost) = &self.ghost else {
            return;
        };
        let fade = ghost.fade.value(cx).clamp(0.0, 1.0);
        if fade <= 0.0 {
            return;
        }
        let size = ghost.size * (0.4 + 0.6 * fade);
        let rect = Rect::new(
            ghost.center.x - size / 2.0,
            ghost.center.y - size / 2.0,
            size,
            size,
        );
        let mut icon = Vec::new();
        match &ghost.icon {
            Some(image) => icon.push(Item::Image {
                rect,
                image: image.clone(),
            }),
            None => generic_icon(cx, rect, ghost.initial.as_ref(), &mut icon),
        }
        items.push(Item::Opacity {
            alpha: fade,
            items: icon,
        });
    }

    fn paint_remove_label(&self, cx: &Context, items: &mut Vec<Item>) {
        let Some(drag) = self.drag.as_ref().filter(|drag| drag.removing) else {
            return;
        };
        let Some((_, text)) = &self.remove_text else {
            return;
        };
        let Some(center) = self
            .slots
            .iter()
            .find(|slot| slot.key == drag.key)
            .and_then(|slot| slot.pinned)
        else {
            return;
        };
        let icon = self.effective_icon(self.along(self.surface));
        let width = (text.width() + 16.0).ceil();
        let height = (text.height() + 6.0).ceil();
        let rect = Rect::new(
            (center.x - width / 2.0).round(),
            (center.y + icon / 2.0 + 6.0).round(),
            width,
            height,
        );
        let shape = Shape::rounded(rect, height / 2.0);
        items.push(Item::Shadow {
            shape,
            offset: Point::new(0.0, 1.0),
            blur_radius: 6.0,
            color: Color::hex(0x000000).with_alpha(70),
        });
        items.push(Item::Fill {
            shape,
            paint: cx.palette.ink.with_alpha(200).into(),
        });
        items.push(Item::Text {
            origin: Point::new(
                rect.x + 8.0,
                (rect.y + (height - text.height()) / 2.0).round(),
            ),
            paragraph: text.clone(),
        });
    }

    pub fn panel_radius(&self) -> f32 {
        if self.rail.is_some() {
            return 0.0;
        }
        (thickness(self.icon_size) * 0.3).round()
    }

    fn paint_panel(&self, cx: &Context, items: &mut Vec<Item>) {
        let dark = self.scheme == Scheme::Dark;
        let radius = self.panel_radius();
        let shape = Shape::rounded(self.panel, radius);
        items.push(Item::Shadow {
            shape,
            offset: Point::new(0.0, 3.0),
            blur_radius: 16.0,
            color: Color::hex(0x000000).with_alpha(if dark { 110 } else { 46 }),
        });
        items.push(Item::Fill {
            shape,
            paint: cx.palette.chassis.into(),
        });
        items.push(Item::Stroke {
            shape: Shape::rounded(self.panel.inset(0.5), radius - 0.5),
            width: 1.0,
            paint: cx.palette.edge.into(),
        });
    }

    fn paint_separator(&self, cx: &Context, cell: Rect, items: &mut Vec<Item>) {
        if let Some(rail) = self.rail {
            let length = if rail.expanded {
                rail.width - 4.0 * ROW_INSET
            } else {
                RAIL_HAIRLINE
            };
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    ((rail.width - length) / 2.0).round(),
                    (cell.y + cell.height / 2.0 - 0.5).round(),
                    length,
                    1.0,
                )),
                paint: cx.palette.hairline.into(),
            });
            return;
        }
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
            paint: cx.palette.ink2.with_alpha(90).into(),
        });
    }

    fn paint_app(
        &self,
        cx: &Context,
        slot: &Slot,
        cell: Rect,
        app_id: &str,
        now: Instant,
        items: &mut Vec<Item>,
    ) {
        let Some(app) = &slot.app else {
            return;
        };
        if self.expanded() && self.active.as_deref() == Some(app_id) {
            items.push(Item::Fill {
                shape: Shape::rounded(
                    Rect::new(
                        cell.x + ROW_INSET,
                        cell.y + 1.0,
                        cell.width - 2.0 * ROW_INSET,
                        cell.height - 2.0,
                    ),
                    ROW_RADIUS,
                ),
                paint: cx.palette.well.into(),
            });
        }
        self.paint_name(slot, cell, items);
        let base = self.icon_rect(cell);
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
            None => generic_icon(cx, rect, slot.initial.as_ref(), &mut icon_items),
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
                paint: cx.palette.warning.into(),
            });
        }
        let dot = slot.dot.value(cx);
        if self.rail.is_some() {
            self.paint_rail_indicators(cx, base, app_id, dot, items);
        } else if dot > 0.0 {
            let center = self.dot_center(cell, base);
            let size = DOT * (0.5 + 0.5 * dot);
            items.push(Item::Opacity {
                alpha: dot.min(1.0),
                items: vec![Item::Fill {
                    shape: Shape::rounded(
                        Rect::new(center.x - size / 2.0, center.y - size / 2.0, size, size),
                        size / 2.0,
                    ),
                    paint: cx.palette.ink.with_alpha(220).into(),
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
                paint: cx.palette.ink.with_alpha(110).into(),
            });
            let fill = Rect::new(track.x, track.y, track.width * progress, track.height);
            items.push(Item::Fill {
                shape: Shape::rounded(fill, PROGRESS_HEIGHT / 2.0),
                paint: cx.palette.accent.into(),
            });
        }
        let badge = slot.badge.value(cx).max(0.0);
        if badge > 0.0
            && let Some((_, text)) = &slot.badge_text
        {
            let width = (text.width() + 10.0).max(BADGE_HEIGHT);
            let rest = if self.expanded() {
                Rect::new(
                    cell.x + cell.width - ROW_INSET - 6.0 - width,
                    cell.y + (cell.height - BADGE_HEIGHT) / 2.0,
                    width,
                    BADGE_HEIGHT,
                )
            } else {
                Rect::new(
                    base.x + base.width - width + 4.0,
                    base.y - 4.0,
                    width,
                    BADGE_HEIGHT,
                )
            };
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
                    paint: cx.palette.danger.into(),
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

    fn paint_name(&self, slot: &Slot, cell: Rect, items: &mut Vec<Item>) {
        let Some((_, _, text)) = slot.name_text.as_ref().filter(|_| self.expanded()) else {
            return;
        };
        let icon = self.icon_rect(cell);
        items.push(Item::Text {
            origin: Point::new(
                (icon.x + icon.width + ROW_TEXT_GAP).round(),
                (cell.y + (cell.height - text.height()) / 2.0).round(),
            ),
            paragraph: text.clone(),
        });
    }

    fn paint_rail_indicators(
        &self,
        cx: &Context,
        icon: Rect,
        app_id: &str,
        running: f32,
        items: &mut Vec<Item>,
    ) {
        let middle = icon.y + icon.height / 2.0;
        if self.active.as_deref() == Some(app_id) && running > 0.0 {
            let (width, height) = RAIL_TICK;
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    0.0,
                    (middle - height / 2.0).round(),
                    width,
                    height,
                )),
                paint: cx.palette.accent.into(),
            });
        } else if running > 0.0 {
            items.push(Item::Opacity {
                alpha: running.min(1.0),
                items: vec![Item::Fill {
                    shape: Shape::rect(Rect::new(
                        1.0,
                        (middle - RAIL_RUNNING / 2.0).round(),
                        RAIL_RUNNING,
                        RAIL_RUNNING,
                    )),
                    paint: cx.palette.ink2.into(),
                }],
            });
        }
    }

    fn paint_more(&self, cx: &Context, items: &mut Vec<Item>) {
        let Some(more) = &self.more else {
            return;
        };
        let icon = self.icon_rect(more.cell);
        items.push(Item::Fill {
            shape: Shape::rounded(icon, RAIL_WELL_RADIUS),
            paint: cx.palette.ink.with_alpha(WELL_ALPHA).into(),
        });
        if let Some((_, _, text)) = &more.text {
            items.push(Item::Text {
                origin: Point::new(
                    (icon.x + (icon.width - text.width()) / 2.0).round(),
                    (icon.y + (icon.height - text.height()) / 2.0).round(),
                ),
                paragraph: text.clone(),
            });
        }
    }

    fn dot_center(&self, cell: Rect, icon: Rect) -> Point {
        let inset = CROSS_PADDING / 2.0;
        match self.edge {
            BarEdge::Left => Point::new(cell.x + inset, icon.y + icon.height / 2.0),
            BarEdge::Right => Point::new(cell.x + cell.width - inset, icon.y + icon.height / 2.0),
            BarEdge::Top => Point::new(icon.x + icon.width / 2.0, cell.y + inset),
            BarEdge::Bottom => Point::new(icon.x + icon.width / 2.0, cell.y + cell.height - inset),
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
            BarEdge::Left => Rect::new(
                panel.x + panel.width + LABEL_GAP,
                along - height / 2.0,
                width,
                height,
            ),
            BarEdge::Right => Rect::new(
                panel.x - LABEL_GAP - width,
                along - height / 2.0,
                width,
                height,
            ),
            BarEdge::Top => Rect::new(
                along - width / 2.0,
                panel.y + panel.height + LABEL_GAP,
                width,
                height,
            ),
            BarEdge::Bottom => Rect::new(
                along - width / 2.0,
                panel.y - LABEL_GAP - height,
                width,
                height,
            ),
        };
        let rect = Rect::new(rect.x.round(), rect.y.round(), rect.width, rect.height);
        let shape = Shape::rounded(rect, height / 2.0);
        let dark = self.scheme == Scheme::Dark;
        let fill = cx.palette.overlay;
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
                    paint: cx.palette.edge.into(),
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

fn generic_icon(cx: &Context, rect: Rect, initial: Option<&Paragraph>, items: &mut Vec<Item>) {
    items.push(Item::Fill {
        shape: Shape::rounded(rect, rect.width * 0.224),
        paint: cx.palette.well.into(),
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

    fn cursor(&self, _point: Point) -> Option<Cursor> {
        self.state
            .borrow()
            .drag
            .as_ref()
            .filter(|drag| drag.key == self.key)
            .map(|_| Cursor::Grabbing)
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
                state.press_at = None;
                if let Some(at) = state.drag.as_ref().map(|drag| drag.at) {
                    state.drop_drag(at, now);
                }
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
                state.press_at(&self.key, pointer.position);
            }
            Event::PointerMove(pointer) if self.state.borrow().is_dragging() => {
                self.state.borrow_mut().drag_to(pointer.position, now);
            }
            Event::PointerMove(pointer) if self.pressed() => {
                if self
                    .state
                    .borrow_mut()
                    .start_drag(&self.key, pointer.position, now)
                {
                    self.press(cx, false, false);
                    return true;
                }
                let inside = contains(self.frame, pointer.position);
                if !inside {
                    self.state.borrow_mut().hold = None;
                }
                self.press(cx, true, inside);
            }
            Event::PointerUp(pointer)
                if pointer.is_primary() && self.state.borrow().is_dragging() =>
            {
                self.state.borrow_mut().drop_drag(pointer.position, now);
            }
            Event::PointerUp(pointer) if pointer.is_primary() => {
                self.state.borrow_mut().press_at = None;
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

struct MoreTile {
    id: WidgetId,
    state: Shared,
    frame: Rect,
}

impl Widget for MoreTile {
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
        let count = state.more.as_ref().map_or(0, |more| more.hidden.len());
        let mut node = Node::new(Role::Button);
        node.set_label(L10N.tr_args("more-apps-name", [("count", (count as i64).into())]));
        node.add_action(Action::Click);
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer)
                if pointer.is_primary() || pointer.button == PointerButton::Secondary =>
            {
                self.state.borrow_mut().open_more();
                true
            }
            _ => false,
        }
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action != Action::Click {
            return false;
        }
        self.state.borrow_mut().open_more();
        true
    }
}

struct Dock {
    id: WidgetId,
    more_id: WidgetId,
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
        state.prepare_more(cx);
        self.children = state
            .slots
            .iter()
            .filter(|slot| !slot.leaving && !slot.hidden && !slot.key.is_separator())
            .map(|slot| {
                Box::new(Tile {
                    id: slot.id,
                    key: slot.key.clone(),
                    state: self.state.clone(),
                    frame: state.reach(slot.cell),
                }) as Box<dyn Widget>
            })
            .collect();
        if let Some(more) = &state.more {
            self.children.push(Box::new(MoreTile {
                id: self.more_id,
                state: self.state.clone(),
                frame: state.reach(more.cell),
            }));
        }
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
            more_id: WidgetId::next(),
            state,
            children: Vec::new(),
            frame: Rect::default(),
        }),
        Context::new(text, palette),
    )
}

pub fn surface_size(edge: BarEdge, icon_size: f32, output: (u32, u32)) -> (u32, u32) {
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
