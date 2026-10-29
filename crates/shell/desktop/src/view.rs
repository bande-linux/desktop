use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
use unbound_files::NameEditor;
use unbound_toolkit::ui::{Animated, Event, Key, Modifiers, Pointer, PointerButton, Spring};
use unbound_toolkit::{
    Appearance, Color, Image, Item as SceneItem, Palette, Paragraph, Point, Rect, Scene, Shape,
    TextAlign, TextStyle, TextSystem,
};

use crate::L10N;
use crate::backdrop::{Backdrop, Layer, placeholder};
use crate::model::{
    Bounds, Desktop, Direction, Grid, Item, Kind, LABEL_WIDTH, Positions, SortBy, icon_bounds,
    label_bounds,
};
use crate::places::Layouts;

const LABEL_SIZE: f32 = 12.0;
const LABEL_LINES: usize = 2;
const KEEP_AT_END: usize = 8;
const DRAG_THRESHOLD: f32 = 4.0;
const TYPE_SELECT_PAUSE: Duration = Duration::from_secs(1);
const EDITOR_HEIGHT: f32 = 18.0;
const EDITOR_WIDTH: f32 = LABEL_WIDTH + 24.0;
const FRAME: Duration = Duration::from_millis(8);
const DARK_TEXT: Color = Color::hex(0x16181C);
const LIGHT_TEXT: Color = Color::hex(0xFFFFFF);
const UNFOCUSED_SELECTION: Color = Color::hex(0x8A9099);
const ROOT_NODE: NodeId = NodeId(1);
const FIRST_ITEM_NODE: u64 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Open(Vec<PathBuf>),
    Trash(Vec<PathBuf>),
    Rename {
        path: PathBuf,
        name: String,
    },
    NewFolder,
    Duplicate(Vec<PathBuf>),
    Transfer {
        sources: Vec<PathBuf>,
        folder: PathBuf,
        copy: bool,
    },
    ShowInFiles(PathBuf),
    OpenDesktopFolder,
    Undo,
    Redo,
}

#[derive(Debug, Clone, PartialEq)]
enum Gesture {
    Idle,
    Pressed {
        at: Point,
        hit: Option<PathBuf>,
        on_label: bool,
        was_sole: bool,
    },
    Band {
        from: Point,
        to: Point,
        keep: BTreeSet<PathBuf>,
    },
    Drag {
        from: Point,
        to: Point,
        paths: Vec<PathBuf>,
        copy: bool,
    },
}

struct Rename {
    path: PathBuf,
    editor: NameEditor,
}

struct Motion {
    x: Animated,
    y: Animated,
    shown: Animated,
}

#[derive(Clone)]
struct Label {
    name: String,
    on_light: bool,
    selected: bool,
    text: Paragraph,
    shadow: Option<Paragraph>,
}

pub struct DesktopView {
    desktop: Desktop,
    layouts: Layouts,
    layouts_changed: bool,
    icons: HashMap<PathBuf, Option<Image>>,
    labels: HashMap<PathBuf, Label>,
    motions: HashMap<PathBuf, Motion>,
    backdrop: Backdrop,
    text: TextSystem,
    appearance: Appearance,
    focused: bool,
    size: (f32, f32),
    scale: f32,
    gesture: Gesture,
    modifiers: Modifiers,
    rename: Option<Rename>,
    rename_when_listed: Option<PathBuf>,
    typed: String,
    typed_at: Option<Instant>,
    populated: bool,
    animating: bool,
    requests: Vec<Request>,
}

fn rect(bounds: Bounds) -> Rect {
    Rect::new(bounds.x, bounds.y, bounds.width, bounds.height)
}

fn fill(bounds: Bounds, radius: f32, color: Color) -> SceneItem {
    SceneItem::Fill {
        shape: Shape::rounded(rect(bounds), radius),
        paint: color.into(),
    }
}

fn distance(a: Point, b: Point) -> f32 {
    (a.x - b.x).hypot(a.y - b.y)
}

pub fn fitted_label(text: &mut TextSystem, name: &str, style: TextStyle, width: f32) -> Paragraph {
    let mut layout = |candidate: &str| {
        let paragraph = text.paragraph(candidate, style, Some(width), TextAlign::Center);
        let fits = paragraph.line_count() <= LABEL_LINES && paragraph.width() <= width + 0.5;
        (paragraph, fits)
    };
    let (full, fits) = layout(name);
    if fits {
        return full;
    }
    let characters: Vec<char> = name.chars().collect();
    let kept = (characters.len() / 3).min(KEEP_AT_END);
    let tail: String = characters[characters.len() - kept..].iter().collect();
    let candidate = |head: usize| {
        let head: String = characters[..head].iter().collect();
        format!("{}…{tail}", head.trim_end())
    };
    let (mut shortest, mut longest) = (0, characters.len() - kept);
    while shortest < longest {
        let middle = (shortest + longest).div_ceil(2);
        if layout(&candidate(middle)).1 {
            shortest = middle;
        } else {
            longest = middle - 1;
        }
    }
    layout(&candidate(shortest)).0
}

impl DesktopView {
    pub fn new(appearance: Appearance, layouts: Layouts) -> Self {
        Self {
            desktop: Desktop::new(Grid::new(Bounds::new(0.0, 0.0, 1.0, 1.0))),
            layouts,
            layouts_changed: false,
            icons: HashMap::new(),
            labels: HashMap::new(),
            motions: HashMap::new(),
            backdrop: Backdrop::default(),
            text: TextSystem::new(),
            appearance,
            focused: false,
            size: (1.0, 1.0),
            scale: 1.0,
            gesture: Gesture::Idle,
            modifiers: Modifiers::default(),
            rename: None,
            rename_when_listed: None,
            typed: String::new(),
            typed_at: None,
            populated: false,
            animating: false,
            requests: Vec::new(),
        }
    }

    pub fn desktop(&self) -> &Desktop {
        &self.desktop
    }

    fn remembered(&self) -> Positions {
        self.layouts
            .get(&self.desktop.grid().key())
            .cloned()
            .unwrap_or_default()
    }

    fn remember(&mut self) {
        let key = self.desktop.grid().key();
        let positions = self.desktop.positions();
        if self.layouts.get(&key) != Some(&positions) {
            self.layouts.insert(key, positions);
            self.layouts_changed = true;
        }
    }

    pub fn take_changed_layouts(&mut self) -> Option<&Layouts> {
        std::mem::take(&mut self.layouts_changed).then_some(&self.layouts)
    }

    pub fn set_viewport(&mut self, size: (f32, f32), scale: f32, area: Bounds, now: Instant) {
        self.size = size;
        self.scale = scale;
        let grid = Grid::new(area);
        if grid != self.desktop.grid() {
            let remembered = self
                .layouts
                .get(&format!("{}x{}", grid.columns, grid.rows))
                .cloned()
                .unwrap_or_default();
            self.desktop.set_grid(grid, &remembered);
            self.labels.clear();
            self.retarget(now);
        }
    }

    pub fn set_items(&mut self, items: Vec<(Item, Option<Image>)>, now: Instant) {
        let remembered = self.remembered();
        self.icons = items
            .iter()
            .map(|(item, icon)| (item.path.clone(), icon.clone()))
            .collect();
        self.desktop.set_items(
            items.into_iter().map(|(item, _)| item).collect(),
            &remembered,
        );
        self.labels.retain(|path, label| {
            self.desktop
                .item(path)
                .is_some_and(|item| item.name == label.name)
        });
        if let Some(path) = self.rename_when_listed.take() {
            match self.desktop.item(&path) {
                Some(_) => {
                    self.desktop.select_only(&path);
                    self.start_rename(&path);
                }
                None => self.rename_when_listed = Some(path),
            }
        }
        if let Some(rename) = &self.rename
            && self.desktop.item(&rename.path).is_none()
        {
            self.rename = None;
        }
        self.retarget(now);
        self.populated = true;
        self.remember();
    }

    pub fn rename_when_listed(&mut self, path: PathBuf) {
        self.rename_when_listed = Some(path);
    }

    pub fn select_when_listed(&mut self, paths: &[PathBuf]) {
        self.desktop.clear_selection();
        for path in paths {
            self.desktop.extend(path);
        }
    }

    pub fn show_wallpaper(&mut self, layer: Layer, now: Instant) {
        self.backdrop.show(layer, now);
        self.labels.clear();
        self.animating = true;
    }

    pub fn wallpaper_key(&self) -> Option<&crate::backdrop::Key> {
        self.backdrop.key()
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.appearance = appearance;
        self.labels.clear();
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        if !focused {
            self.commit_rename();
            self.gesture = Gesture::Idle;
        }
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn deadline(&self, now: Instant) -> Option<Instant> {
        let typing = self.typed_at.map(|at| at + TYPE_SELECT_PAUSE);
        let frame = self.animating.then(|| now + FRAME);
        [typing, frame].into_iter().flatten().min()
    }

    pub fn tick(&mut self, now: Instant) {
        if self
            .typed_at
            .is_some_and(|at| now >= at + TYPE_SELECT_PAUSE)
        {
            self.typed.clear();
            self.typed_at = None;
        }
    }

    pub fn arrange(&mut self, by: SortBy, now: Instant) {
        self.desktop.arrange(by);
        self.retarget(now);
        self.remember();
    }

    pub fn clean_up(&mut self, now: Instant) {
        self.desktop.clean_up();
        self.retarget(now);
        self.remember();
    }

    fn retarget(&mut self, now: Instant) {
        let grid = self.desktop.grid();
        let reduce_motion = self.appearance.reduce_motion;
        let appear = self.populated;
        let mut present = BTreeSet::new();
        for (item, slot) in self.desktop.items() {
            let cell = grid.cell(slot);
            present.insert(item.path.clone());
            let motion = self.motions.entry(item.path.clone()).or_insert_with(|| {
                let mut shown = Animated::new(if appear { 0.0 } else { 1.0 });
                shown.animate_at(now, reduce_motion, 1.0, Spring::STANDARD);
                Motion {
                    x: Animated::new(cell.x),
                    y: Animated::new(cell.y),
                    shown,
                }
            });
            motion
                .x
                .animate_at(now, reduce_motion, cell.x, Spring::STANDARD);
            motion
                .y
                .animate_at(now, reduce_motion, cell.y, Spring::STANDARD);
        }
        self.motions.retain(|path, _| present.contains(path));
        self.animating = true;
    }

    fn start_rename(&mut self, path: &Path) {
        let Some(item) = self.desktop.item(path) else {
            return;
        };
        if item.kind == Kind::Volume {
            return;
        }
        self.rename = Some(Rename {
            path: path.to_path_buf(),
            editor: NameEditor::new(&item.name, item.kind == Kind::Folder),
        });
    }

    fn commit_rename(&mut self) {
        let Some(rename) = self.rename.take() else {
            return;
        };
        let name = rename.editor.text().trim().to_owned();
        let unchanged = self
            .desktop
            .item(&rename.path)
            .is_some_and(|item| item.name == name);
        if !name.is_empty() && !unchanged {
            self.requests.push(Request::Rename {
                path: rename.path,
                name,
            });
        }
    }

    fn editor_bounds(&self, path: &Path) -> Option<Bounds> {
        let slot = self.desktop.slot_of(path)?;
        let label = label_bounds(self.desktop.grid().cell(slot));
        Some(Bounds::new(
            label.x + (label.width - EDITOR_WIDTH) / 2.0,
            label.y - 1.0,
            EDITOR_WIDTH,
            EDITOR_HEIGHT,
        ))
    }

    pub fn handle(&mut self, event: &Event, now: Instant) -> bool {
        match event {
            Event::PointerDown(pointer) => self.pointer_down(pointer),
            Event::PointerMove(pointer) => self.pointer_move(pointer),
            Event::PointerUp(pointer) => self.pointer_up(pointer, now),
            Event::ModifiersChanged(modifiers) => {
                self.modifiers = *modifiers;
                if let Gesture::Drag { copy, .. } = &mut self.gesture {
                    *copy = modifiers.alt;
                }
                true
            }
            Event::Key { key, modifiers } => self.key(*key, *modifiers, now),
            Event::Commit(text) => match &mut self.rename {
                Some(rename) => {
                    rename.editor.insert(text);
                    true
                }
                None => {
                    self.type_select(text, now);
                    true
                }
            },
            _ => false,
        }
    }

    fn pointer_down(&mut self, pointer: &Pointer) -> bool {
        let at = pointer.position;
        if let Some(rename) = &self.rename {
            let inside = self
                .editor_bounds(&rename.path)
                .is_some_and(|bounds| bounds.contains(at.x, at.y));
            if inside {
                return false;
            }
            self.commit_rename();
        }
        let hit = self.desktop.hit(at.x, at.y).map(|item| item.path.clone());
        let modifiers = pointer.modifiers;
        if pointer.button == PointerButton::Secondary
            || pointer.button == PointerButton::Primary && modifiers.control
        {
            match &hit {
                Some(path) if !self.desktop.is_selected(path) => self.desktop.select_only(path),
                Some(_) => {}
                None => self.desktop.clear_selection(),
            }
            return true;
        }
        if !pointer.is_primary() {
            return false;
        }
        match hit {
            Some(path) => {
                if pointer.count >= 2 && !modifiers.logo && !modifiers.shift {
                    self.desktop.select_only(&path);
                    self.requests.push(Request::Open(vec![path]));
                    self.gesture = Gesture::Idle;
                    return true;
                }
                let was_sole = self.desktop.selected() == [path.clone()];
                if modifiers.logo {
                    self.desktop.toggle(&path);
                } else if modifiers.shift {
                    self.desktop.extend(&path);
                } else if !self.desktop.is_selected(&path) {
                    self.desktop.select_only(&path);
                }
                let on_label = self
                    .desktop
                    .slot_of(&path)
                    .map(|slot| label_bounds(self.desktop.grid().cell(slot)))
                    .is_some_and(|label| label.contains(at.x, at.y));
                self.gesture = Gesture::Pressed {
                    at,
                    hit: Some(path),
                    on_label,
                    was_sole: was_sole && !modifiers.logo && !modifiers.shift,
                };
            }
            None => {
                let keep: BTreeSet<PathBuf> = if modifiers.logo || modifiers.shift {
                    self.desktop.selected().into_iter().collect()
                } else {
                    self.desktop.clear_selection();
                    BTreeSet::new()
                };
                self.gesture = Gesture::Band {
                    from: at,
                    to: at,
                    keep,
                };
            }
        }
        self.typed.clear();
        true
    }

    fn pointer_move(&mut self, pointer: &Pointer) -> bool {
        let at = pointer.position;
        match &mut self.gesture {
            Gesture::Idle => false,
            Gesture::Pressed { at: from, hit, .. } => {
                if hit.is_none() || distance(*from, at) < DRAG_THRESHOLD {
                    return false;
                }
                self.gesture = Gesture::Drag {
                    from: *from,
                    to: at,
                    paths: self.desktop.selected(),
                    copy: pointer.modifiers.alt,
                };
                true
            }
            Gesture::Band { from, to, keep } => {
                *to = at;
                let area = Bounds::spanning((from.x, from.y), (at.x, at.y));
                let keep = keep.clone();
                self.desktop.select_in(area, &keep);
                true
            }
            Gesture::Drag { to, copy, .. } => {
                *to = at;
                *copy = pointer.modifiers.alt;
                true
            }
        }
    }

    fn pointer_up(&mut self, pointer: &Pointer, now: Instant) -> bool {
        if !pointer.is_primary() {
            return false;
        }
        match std::mem::replace(&mut self.gesture, Gesture::Idle) {
            Gesture::Idle => false,
            Gesture::Pressed {
                hit: Some(path),
                on_label,
                was_sole,
                ..
            } => {
                let modifiers = pointer.modifiers;
                if was_sole && on_label {
                    self.start_rename(&path);
                } else if !modifiers.logo && !modifiers.shift {
                    self.desktop.select_only(&path);
                }
                true
            }
            Gesture::Pressed { hit: None, .. } | Gesture::Band { .. } => true,
            Gesture::Drag {
                from, to, paths, ..
            } => {
                self.drop_items(from, to, paths, pointer.modifiers.alt, now);
                true
            }
        }
    }

    fn drop_items(
        &mut self,
        from: Point,
        to: Point,
        paths: Vec<PathBuf>,
        copy: bool,
        now: Instant,
    ) {
        let target = self
            .desktop
            .hit(to.x, to.y)
            .filter(|item| matches!(item.kind, Kind::Folder | Kind::Volume))
            .filter(|item| !paths.contains(&item.path))
            .map(|item| item.path.clone());
        if let Some(folder) = target {
            self.requests.push(Request::Transfer {
                sources: paths,
                folder,
                copy,
            });
            return;
        }
        let grid = self.desktop.grid();
        let (start, end) = (grid.slot_at(from.x, from.y), grid.slot_at(to.x, to.y));
        let by = (
            i64::from(end.column) - i64::from(start.column),
            i64::from(end.row) - i64::from(start.row),
        );
        self.desktop.move_items(&paths, by);
        self.retarget(now);
        self.remember();
    }

    fn type_select(&mut self, text: &str, now: Instant) {
        if self
            .typed_at
            .is_some_and(|at| now >= at + TYPE_SELECT_PAUSE)
        {
            self.typed.clear();
        }
        self.typed.push_str(text);
        self.typed_at = Some(now);
        self.desktop.select_prefix(&self.typed);
    }

    fn rename_key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        match key {
            Key::Enter => self.commit_rename(),
            Key::Escape => self.rename = None,
            Key::Tab => {
                self.commit_rename();
            }
            _ => {
                if let Some(rename) = &mut self.rename {
                    return rename.editor.key(key, modifiers);
                }
            }
        }
        true
    }

    fn key(&mut self, key: Key, modifiers: Modifiers, now: Instant) -> bool {
        if self.rename.is_some() {
            return self.rename_key(key, modifiers);
        }
        let selected = self.desktop.selected();
        let command = modifiers.logo;
        match key {
            Key::Down if command => {
                if !selected.is_empty() {
                    self.requests.push(Request::Open(selected));
                }
            }
            Key::Left => self.desktop.navigate(Direction::Left, modifiers.shift),
            Key::Right => self.desktop.navigate(Direction::Right, modifiers.shift),
            Key::Up => self.desktop.navigate(Direction::Up, modifiers.shift),
            Key::Down => self.desktop.navigate(Direction::Down, modifiers.shift),
            Key::Enter if !selected.is_empty() => self.requests.push(Request::Open(selected)),
            Key::Function(2) => {
                if let [path] = selected.as_slice() {
                    self.start_rename(path);
                }
            }
            Key::Backspace | Key::Delete if command && !selected.is_empty() => {
                self.requests.push(Request::Trash(selected));
            }
            Key::Escape => {
                self.gesture = Gesture::Idle;
                self.desktop.clear_selection();
            }
            Key::Character(letter) if command => match letter.to_ascii_lowercase() {
                'a' => self.desktop.select_all(),
                'n' if modifiers.shift => self.requests.push(Request::NewFolder),
                'd' if modifiers.shift => self.requests.push(Request::OpenDesktopFolder),
                'd' if !selected.is_empty() => self.requests.push(Request::Duplicate(selected)),
                'i' => {
                    if let Some(path) = selected.first() {
                        self.requests.push(Request::ShowInFiles(path.clone()));
                    }
                }
                'z' if modifiers.shift => self.requests.push(Request::Redo),
                'z' => self.requests.push(Request::Undo),
                _ => return false,
            },
            Key::Character(letter) if !modifiers.control && !modifiers.alt => {
                self.type_select(letter.encode_utf8(&mut [0; 4]), now);
            }
            _ => return false,
        }
        true
    }

    fn label(&mut self, item: &Item, bounds: Bounds, selected: bool) -> Label {
        let on_light = !selected
            && self
                .backdrop
                .is_light(bounds, self.scale)
                .unwrap_or(self.appearance.scheme == unbound_toolkit::Scheme::Light);
        if let Some(label) = self.labels.get(&item.path)
            && label.name == item.name
            && label.on_light == on_light
            && label.selected == selected
        {
            return label.clone();
        }
        let (color, shadow) = match (selected, on_light) {
            (true, _) => (LIGHT_TEXT, None),
            (false, true) => (DARK_TEXT, Some(LIGHT_TEXT.with_alpha(150))),
            (false, false) => (LIGHT_TEXT, Some(Color::hex(0).with_alpha(150))),
        };
        let style = TextStyle::new(LABEL_SIZE, color).weight(TextStyle::MEDIUM);
        let text = fitted_label(&mut self.text, &item.name, style, LABEL_WIDTH);
        let shadow = shadow.map(|shadow| {
            fitted_label(
                &mut self.text,
                &item.name,
                TextStyle {
                    color: shadow,
                    ..style
                },
                LABEL_WIDTH,
            )
        });
        let label = Label {
            name: item.name.clone(),
            on_light,
            selected,
            text,
            shadow,
        };
        self.labels.insert(item.path.clone(), label.clone());
        label
    }

    fn paint_item(
        &mut self,
        item: &Item,
        cell: Bounds,
        shown: f32,
        palette: Palette,
        items: &mut Vec<SceneItem>,
    ) {
        let selected = self.desktop.is_selected(&item.path);
        let icon = icon_bounds(cell);
        let mut drawn = Vec::new();
        if selected {
            let backdrop = Bounds::new(
                icon.x - 5.0,
                icon.y - 5.0,
                icon.width + 10.0,
                icon.height + 10.0,
            );
            drawn.push(fill(backdrop, 9.0, Color::hex(0).with_alpha(56)));
        }
        let grow = 0.85 + 0.15 * shown;
        let scaled = Bounds::new(
            icon.x + icon.width * (1.0 - grow) / 2.0,
            icon.y + icon.height * (1.0 - grow) / 2.0,
            icon.width * grow,
            icon.height * grow,
        );
        match self.icons.get(&item.path).cloned().flatten() {
            Some(image) => drawn.push(SceneItem::Image {
                rect: rect(scaled),
                image,
            }),
            None => drawn.push(fill(scaled.inset(8.0), 8.0, palette.surface_elevated)),
        }
        let renaming = self
            .rename
            .as_ref()
            .is_some_and(|rename| rename.path == item.path);
        if !renaming {
            let area = label_bounds(cell);
            let label = self.label(item, area, selected);
            let origin = Point::new(area.x.round(), area.y.round());
            if selected {
                let pill = Bounds::new(
                    (area.x + (area.width - label.text.width()) / 2.0).round() - 4.0,
                    origin.y - 1.0,
                    label.text.width() + 8.0,
                    label.text.height() + 2.0,
                );
                let color = if self.focused {
                    palette.accent_color
                } else {
                    UNFOCUSED_SELECTION
                };
                drawn.push(fill(pill, 5.0, color));
            }
            if let Some(shadow) = label.shadow {
                drawn.push(SceneItem::Text {
                    origin: Point::new(origin.x, origin.y + 1.0),
                    paragraph: shadow,
                });
            }
            drawn.push(SceneItem::Text {
                origin,
                paragraph: label.text,
            });
        }
        if shown >= 1.0 {
            items.extend(drawn);
        } else {
            items.push(SceneItem::Opacity {
                alpha: shown.clamp(0.0, 1.0),
                items: drawn,
            });
        }
    }

    fn paint_editor(&mut self, palette: Palette, items: &mut Vec<SceneItem>) {
        let Some(rename) = &self.rename else {
            return;
        };
        let Some(field) = self.editor_bounds(&rename.path) else {
            return;
        };
        let text = rename.editor.text().to_owned();
        let selection = rename.editor.selection();
        let cursor = rename.editor.cursor();
        let style = TextStyle::new(LABEL_SIZE, palette.label_primary).weight(TextStyle::MEDIUM);
        let measure = |system: &mut TextSystem, upto: usize| {
            system
                .paragraph(&text[..upto], style, None, TextAlign::Start)
                .width()
        };
        let full = measure(&mut self.text, text.len());
        let (start, end, caret) = (
            measure(&mut self.text, selection.start),
            measure(&mut self.text, selection.end),
            measure(&mut self.text, cursor),
        );
        let inner = field.width - 8.0;
        let scroll = if full <= inner {
            -((inner - full) / 2.0).round()
        } else {
            (caret - inner + 2.0).max(0.0)
        };
        let text_x = field.x + 4.0 - scroll;
        items.push(SceneItem::Shadow {
            shape: Shape::rounded(rect(field), 4.0),
            offset: Point::new(0.0, 1.0),
            blur_radius: 4.0,
            color: Color::hex(0).with_alpha(60),
        });
        items.push(fill(field, 4.0, palette.surface_elevated));
        items.push(SceneItem::Stroke {
            shape: Shape::rounded(rect(field), 4.0),
            width: 1.5,
            paint: palette.accent_color.into(),
        });
        let mut inside = Vec::new();
        if selection.is_empty() {
            inside.push(fill(
                Bounds::new(text_x + caret, field.y + 3.0, 1.0, field.height - 6.0),
                0.0,
                palette.label_primary,
            ));
        } else {
            inside.push(fill(
                Bounds::new(
                    text_x + start,
                    field.y + 2.0,
                    end - start,
                    field.height - 4.0,
                ),
                2.0,
                palette.accent_color.with_alpha(90),
            ));
        }
        let paragraph = self.text.paragraph(&text, style, None, TextAlign::Start);
        inside.push(SceneItem::Text {
            origin: Point::new(text_x, field.y + (field.height - paragraph.height()) / 2.0),
            paragraph,
        });
        items.push(SceneItem::Clip {
            shape: Shape::rect(rect(Bounds::new(
                field.x + 2.0,
                field.y,
                field.width - 4.0,
                field.height,
            ))),
            items: inside,
        });
    }

    pub fn scene(&mut self, now: Instant) -> Scene {
        let palette = self.appearance.palette();
        let dark = self.appearance.scheme == unbound_toolkit::Scheme::Dark;
        let (width, height) = self.size;
        let mut items = Vec::new();
        self.backdrop
            .paint(Rect::new(0.0, 0.0, width, height), now, &mut items);
        let mut animating = self.backdrop.is_fading(now);
        let dragging: Option<(Vec<PathBuf>, Point)> = match &self.gesture {
            Gesture::Drag {
                from, to, paths, ..
            } => Some((paths.clone(), Point::new(to.x - from.x, to.y - from.y))),
            _ => None,
        };
        let entries: Vec<(Item, Bounds, f32)> = self
            .desktop
            .items()
            .map(|(item, slot)| {
                let cell = self.desktop.grid().cell(slot);
                let (cell, shown) = match self.motions.get(&item.path) {
                    Some(motion) => {
                        let (x, moving_x) = motion.x.value_at(now);
                        let (y, moving_y) = motion.y.value_at(now);
                        let (shown, appearing) = motion.shown.value_at(now);
                        animating |= moving_x || moving_y || appearing;
                        (Bounds { x, y, ..cell }, shown)
                    }
                    None => (cell, 1.0),
                };
                (item.clone(), cell, shown)
            })
            .collect();
        let mut lifted = Vec::new();
        for (item, cell, shown) in &entries {
            match &dragging {
                Some((paths, _)) if paths.contains(&item.path) => {
                    self.paint_item(item, *cell, *shown, palette, &mut lifted);
                }
                _ => self.paint_item(item, *cell, *shown, palette, &mut items),
            }
        }
        if let Some((_, offset)) = dragging {
            items.push(SceneItem::Opacity {
                alpha: 0.75,
                items: vec![SceneItem::Translate {
                    offset,
                    items: lifted,
                }],
            });
        }
        if let Gesture::Band { from, to, .. } = &self.gesture {
            let band = Bounds::spanning((from.x, from.y), (to.x, to.y));
            items.push(fill(band, 2.0, Color::hex(0xFFFFFF).with_alpha(40)));
            items.push(SceneItem::Stroke {
                shape: Shape::rounded(rect(band), 2.0),
                width: 1.0,
                paint: Color::hex(0xFFFFFF).with_alpha(170).into(),
            });
        }
        self.paint_editor(palette, &mut items);
        self.animating = animating;
        Scene {
            width: width.round() as u32,
            height: height.round() as u32,
            background: placeholder(dark),
            items,
        }
    }

    fn item_node(index: usize) -> NodeId {
        NodeId(FIRST_ITEM_NODE + index as u64)
    }

    pub fn accessibility(&self) -> TreeUpdate {
        let mut root = Node::new(Role::Window);
        root.set_label(L10N.tr("desktop"));
        let mut nodes = Vec::new();
        let mut focus = ROOT_NODE;
        let mut children = Vec::new();
        for (index, (item, slot)) in self.desktop.items().enumerate() {
            let id = Self::item_node(index);
            let cell = self.desktop.grid().cell(slot);
            let mut node = Node::new(Role::ListBoxOption);
            node.set_label(item.name.clone());
            node.set_description(item.kind_name.clone());
            node.set_selected(self.desktop.is_selected(&item.path));
            node.set_bounds(accesskit::Rect {
                x0: f64::from(cell.x),
                y0: f64::from(cell.y),
                x1: f64::from(cell.x + cell.width),
                y1: f64::from(cell.y + cell.height),
            });
            node.add_action(Action::Click);
            node.add_action(Action::Focus);
            if self.desktop.cursor() == Some(item.path.as_path()) {
                focus = id;
            }
            children.push(id);
            nodes.push((id, node));
        }
        root.set_children(children);
        nodes.insert(0, (ROOT_NODE, root));
        TreeUpdate {
            nodes,
            tree: Some(TreeInfo {
                toolkit_name: Some("unbound-toolkit".to_owned()),
                toolkit_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
                ..TreeInfo::new(ROOT_NODE)
            }),
            tree_id: TreeId::ROOT,
            focus,
        }
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        let Some(index) = request.target_node.0.checked_sub(FIRST_ITEM_NODE) else {
            return false;
        };
        let Some(path) = self
            .desktop
            .items()
            .nth(index as usize)
            .map(|(item, _)| item.path.clone())
        else {
            return false;
        };
        match request.action {
            Action::Click => {
                self.desktop.select_only(&path);
                self.requests.push(Request::Open(vec![path]));
            }
            Action::Focus => self.desktop.select_only(&path),
            _ => return false,
        }
        true
    }
}
