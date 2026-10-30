use std::cell::Cell;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
use unbound_toolkit::ui::{
    Alert, Animated, ClipboardRequest, Context, Event, Key, Modifiers, Pointer, PointerButton,
    Size, Spring, TextField, Ui, Widget, WidgetId,
};
use unbound_toolkit::{
    Appearance, Color, Image, Item as SceneItem, Palette, Paragraph, Point, Rect, Scene, Shape,
    TextAlign, TextStyle, TextSystem,
};

use crate::L10N;
use crate::backdrop::{Backdrop, Layer, placeholder};
use crate::editor::{Anchor, Inset};
use crate::menus::Command;
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

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Open(Vec<PathBuf>),
    Trash(Vec<PathBuf>),
    Delete(Vec<PathBuf>),
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
    ShowInfo(PathBuf),
    OpenDesktopFolder,
    Undo,
    Redo,
    ContextMenu {
        at: Point,
        on_items: bool,
    },
    OpenWith {
        app: String,
        paths: Vec<PathBuf>,
    },
    Copy(Vec<PathBuf>),
    Cut(Vec<PathBuf>),
    Paste {
        moves: bool,
    },
    CopyText(String),
    PasteText,
    Eject(Vec<PathBuf>),
    SetWallpaper(PathBuf),
    ChangeWallpaper,
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
    field: WidgetId,
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
    ui: Ui,
    anchor: WidgetId,
    editing: bool,
    deleting: Option<(Vec<PathBuf>, Rc<Cell<bool>>)>,
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
        let anchor = Anchor::new();
        let anchor_id = anchor.id();
        let mut ui = Ui::new(
            Box::new(anchor),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        Self {
            desktop: Desktop::new(Grid::new(Bounds::new(0.0, 0.0, 1.0, 1.0))),
            layouts,
            layouts_changed: false,
            icons: HashMap::new(),
            labels: HashMap::new(),
            motions: HashMap::new(),
            backdrop: Backdrop::default(),
            ui,
            anchor: anchor_id,
            editing: false,
            deleting: None,
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
            self.cancel_rename();
        }
        self.retarget(now);
        self.populated = true;
        self.remember();
    }

    pub fn rename_when_listed(&mut self, path: PathBuf) {
        self.rename_when_listed = Some(path);
    }

    pub fn select_paths(&mut self, paths: &[PathBuf]) {
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
        self.ui.set_appearance(appearance);
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
        let name = item.name.clone();
        let stem = match item.kind {
            Kind::File => Path::new(&name)
                .file_stem()
                .map_or(name.len(), |stem| stem.len()),
            _ => name.len(),
        };
        let mut field = TextField::new("", &name, |_| {}).borderless(LABEL_SIZE);
        field.select(0..stem);
        let field_id = field.id();
        let at = self.editor_bounds(path).map(rect).unwrap_or_default();
        if let Some(anchor) = self.ui.widget_as::<Anchor>(self.anchor) {
            anchor.hold(Some(Box::new(field)), at);
        }
        self.ui.perform(&ActionRequest {
            action: Action::Focus,
            target_tree: TreeId::ROOT,
            target_node: field_id.into(),
            data: None,
        });
        self.ui.layout(Size::new(self.size.0, self.size.1));
        self.rename = Some(Rename {
            path: path.to_path_buf(),
            field: field_id,
        });
    }

    fn cancel_rename(&mut self) {
        self.rename = None;
        self.editing = false;
        if let Some(anchor) = self.ui.widget_as::<Anchor>(self.anchor) {
            anchor.hold(None, Rect::default());
        }
    }

    fn commit_rename(&mut self) {
        let Some(rename) = self.rename.as_ref() else {
            return;
        };
        let path = rename.path.clone();
        let name = self
            .ui
            .widget_as::<TextField>(rename.field)
            .map(|field| field.text().trim().to_owned())
            .unwrap_or_default();
        self.cancel_rename();
        let unchanged = self
            .desktop
            .item(&path)
            .is_some_and(|item| item.name == name);
        if !name.is_empty() && !unchanged {
            self.requests.push(Request::Rename { path, name });
        }
    }

    fn edit(&mut self, event: &Event) -> bool {
        let handled = self.ui.handle(event);
        match self.ui.take_clipboard_request() {
            Some(ClipboardRequest::Copy(text)) => self.requests.push(Request::CopyText(text)),
            Some(ClipboardRequest::Paste) => self.requests.push(Request::PasteText),
            None => {}
        }
        if !self.ui.has_sheet()
            && let Some((paths, confirmed)) = self.deleting.take()
            && confirmed.get()
        {
            self.requests.push(Request::Delete(paths));
        }
        handled
    }

    fn cut(&mut self, paths: Vec<PathBuf>) {
        let files = paths
            .into_iter()
            .filter(|path| {
                self.desktop
                    .item(path)
                    .is_some_and(|item| item.kind != Kind::Volume)
            })
            .collect::<Vec<_>>();
        if !files.is_empty() {
            self.requests.push(Request::Cut(files));
        }
    }

    fn ask_to_delete(&mut self, paths: Vec<PathBuf>) {
        let deletable = paths
            .into_iter()
            .filter(|path| {
                self.desktop
                    .item(path)
                    .is_some_and(|item| item.kind != Kind::Volume)
            })
            .collect::<Vec<_>>();
        if deletable.is_empty() || self.ui.has_sheet() {
            return;
        }
        self.cancel_rename();
        let confirmed = Rc::new(Cell::new(false));
        let chosen = confirmed.clone();
        let alert = Alert::new(
            &L10N.tr_args("delete-title", [("count", deletable.len().into())]),
            &L10N.tr("cannot-undo"),
            move |action| chosen.set(action == "delete"),
        )
        .cancel(&L10N.tr("cancel"), "cancel")
        .primary(&L10N.tr("delete"), "delete");
        self.ui
            .present_sheet(Inset::new(self.desktop.grid().area.y, alert));
        self.deleting = Some((deletable, confirmed));
        self.gesture = Gesture::Idle;
        self.animating = true;
    }

    pub fn pasted(&mut self, text: &str) -> bool {
        self.ui.pasted(text)
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
        if self.ui.has_sheet() {
            self.edit(event);
            return true;
        }
        match event {
            Event::PointerDown(pointer) => self.pointer_down(event, pointer),
            Event::PointerMove(_) | Event::PointerUp(_) if self.editing => {
                if matches!(event, Event::PointerUp(_)) {
                    self.editing = false;
                }
                self.edit(event)
            }
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
            Event::Commit(_) | Event::Preedit { .. } if self.rename.is_some() => self.edit(event),
            Event::Commit(text) => {
                self.type_select(text, now);
                true
            }
            _ => false,
        }
    }

    fn pointer_down(&mut self, event: &Event, pointer: &Pointer) -> bool {
        let at = pointer.position;
        if let Some(rename) = &self.rename {
            let inside = self
                .editor_bounds(&rename.path)
                .is_some_and(|bounds| bounds.contains(at.x, at.y));
            if inside {
                self.editing = true;
                return self.edit(event);
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
            self.gesture = Gesture::Idle;
            self.requests.push(Request::ContextMenu {
                at,
                on_items: hit.is_some(),
            });
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
            Key::Escape => self.cancel_rename(),
            Key::Tab => {
                self.commit_rename();
            }
            _ => return self.edit(&Event::Key { key, modifiers }),
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
            Key::Backspace | Key::Delete if command && modifiers.alt && !selected.is_empty() => {
                self.ask_to_delete(selected);
            }
            Key::Backspace | Key::Delete if command && !selected.is_empty() => {
                self.requests.push(Request::Trash(selected));
            }
            Key::Function(10) if modifiers.shift => self.menu_from_keyboard(),
            Key::Escape => {
                self.gesture = Gesture::Idle;
                self.desktop.clear_selection();
            }
            Key::Character(letter) if command => match letter.to_ascii_lowercase() {
                'a' => self.desktop.select_all(),
                'n' if modifiers.shift => self.requests.push(Request::NewFolder),
                'd' if modifiers.shift => self.requests.push(Request::OpenDesktopFolder),
                'd' if !selected.is_empty() => self.requests.push(Request::Duplicate(selected)),
                'c' if !selected.is_empty() => self.requests.push(Request::Copy(selected)),
                'x' if !selected.is_empty() => self.cut(selected),
                'v' => self.requests.push(Request::Paste {
                    moves: modifiers.alt,
                }),
                'i' => {
                    if let Some(path) = selected.first() {
                        self.requests.push(Request::ShowInfo(path.clone()));
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

    fn menu_from_keyboard(&mut self) {
        let at = self
            .desktop
            .cursor()
            .filter(|cursor| self.desktop.is_selected(cursor))
            .and_then(|cursor| self.desktop.slot_of(cursor))
            .map(|slot| {
                let label = label_bounds(self.desktop.grid().cell(slot));
                Point::new(label.x + label.width / 2.0, label.y)
            });
        let on_items = at.is_some() && !self.desktop.selected().is_empty();
        let area = self.desktop.grid().area;
        let at = at.unwrap_or(Point::new(
            area.x + area.width / 2.0,
            area.y + area.height / 2.0,
        ));
        self.requests.push(Request::ContextMenu { at, on_items });
    }

    pub fn run_command(&mut self, command: Command, now: Instant) {
        let selected = self.desktop.selected();
        let request = match command {
            Command::Sort(by) => {
                self.arrange(by, now);
                return;
            }
            Command::CleanUp => {
                self.clean_up(now);
                return;
            }
            Command::Rename => {
                if let [path] = selected.as_slice() {
                    self.start_rename(path);
                }
                return;
            }
            Command::NewFolder => Request::NewFolder,
            Command::DesktopInfo | Command::ShowDesktopFolder => Request::OpenDesktopFolder,
            Command::ChangeWallpaper => Request::ChangeWallpaper,
            Command::Open => Request::Open(selected),
            Command::OpenWith(app) => Request::OpenWith {
                app,
                paths: selected,
            },
            Command::Info => match selected.first() {
                Some(path) => Request::ShowInfo(path.clone()),
                None => return,
            },
            Command::Duplicate => Request::Duplicate(selected),
            Command::Copy => Request::Copy(selected),
            Command::Cut => {
                self.cut(selected);
                return;
            }
            Command::Paste => Request::Paste { moves: false },
            Command::Trash => Request::Trash(selected),
            Command::DeleteImmediately => {
                self.ask_to_delete(selected);
                return;
            }
            Command::Eject => Request::Eject(selected),
            Command::SetWallpaper => match selected.first() {
                Some(path) => Request::SetWallpaper(path.clone()),
                None => return,
            },
        };
        self.requests.push(request);
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
        let text = fitted_label(&mut self.ui.context().text, &item.name, style, LABEL_WIDTH);
        let shadow = shadow.map(|shadow| {
            fitted_label(
                &mut self.ui.context().text,
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
        if let Some(anchor) = self.ui.widget_as::<Anchor>(self.anchor) {
            anchor.move_to(rect(field));
        }
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
        items.extend(
            self.ui
                .scene(width.round() as u32, height.round() as u32)
                .items,
        );
        self.animating = animating || self.ui.is_animating();
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
