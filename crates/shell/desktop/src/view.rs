use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
use bande_fs::same_volume;
use bande_toolkit::ui::{
    Animated, ClipboardRequest, Context, Event, FreeLayout, Key, Modifiers, Pointer, PointerButton,
    Size, Spring, TextField, Ui, Widget, WidgetId,
};
use bande_toolkit::{
    Appearance, Color, DragAction, DragIcon, Elevation, Image, Item as SceneItem, Palette,
    Paragraph, Point, Radius, Rect, Scene, Scheme, Shape, TextAlign, TextStyle, TextSystem,
};

use crate::L10N;
use crate::backdrop::{Backdrop, Layer};
use crate::menus::Command;
use crate::model::{
    Bounds, Desktop, Direction, Grid, Item, Kind, LABEL_WIDTH, Positions, Slot, SortBy,
    icon_bounds, label_bounds,
};
use crate::places::Layouts;
use bande_xdg::path_from_file_uri;

const LABEL_SIZE: f32 = 12.0;
const LABEL_LINES: usize = 2;
const KEEP_AT_END: usize = 8;
const DRAG_THRESHOLD: f32 = 4.0;
const TYPE_SELECT_PAUSE: Duration = Duration::from_secs(1);
const EDITOR_HEIGHT: f32 = 18.0;
const EDITOR_WIDTH: f32 = LABEL_WIDTH + 24.0;
const SPRING_DELAY: Duration = Duration::from_millis(900);
const LABEL_HALO: f32 = 0.6;
const DRAGGED: f32 = 0.75;
const BADGE_TEXT: f32 = 11.0;
const BADGE_HEIGHT: f32 = 18.0;
const BADGE_PADDING: f32 = 5.0;
const BADGE_OFFSET: (f32, f32) = (22.0, -14.0);
const THUMBNAIL_INSET: f32 = 6.0;
const THUMBNAIL_RADIUS: f32 = 2.0;
const THUMBNAIL_EDGE: f32 = 0.55;
const LIFTED: f32 = 0.4;
const DROP_TARGET: f32 = 0.35;
const BAND_FILL: f32 = 0.18;
const ROOT_NODE: NodeId = NodeId(1);
const FIRST_ITEM_NODE: u64 = 2;

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Open(Vec<PathBuf>),
    Trash(Vec<PathBuf>),
    ConfirmDelete(Vec<PathBuf>),
    Rename {
        path: PathBuf,
        name: String,
    },
    NewFolder,
    Duplicate(Vec<PathBuf>),
    StartDrag(Vec<PathBuf>),
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
    Clipboard(ClipboardRequest),
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
        paths: Vec<PathBuf>,
    },
}

#[derive(Debug, Clone, PartialEq)]
enum Target {
    Folder(PathBuf),
    Desktop,
}

struct Hover {
    target: Target,
    action: Option<DragAction>,
    since: Instant,
    sprung: bool,
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

#[derive(Debug, Clone, PartialEq)]
pub enum Artwork {
    Icon(Image),
    Thumbnail(Image),
}

#[derive(Clone)]
struct Label {
    name: String,
    on_light: bool,
    selected: Option<bool>,
    text: Paragraph,
    shadow: Option<Paragraph>,
}

pub struct DesktopView {
    desktop: Desktop,
    layouts: Layouts,
    layouts_changed: bool,
    icons: HashMap<PathBuf, Option<Artwork>>,
    labels: HashMap<PathBuf, Label>,
    motions: HashMap<PathBuf, Motion>,
    backdrop: Backdrop,
    wallpaper_unavailable: bool,
    ui: Ui,
    layout: WidgetId,
    editor: Option<WidgetId>,
    editing: bool,
    folder: PathBuf,
    drop: Option<Hover>,
    offered: Vec<PathBuf>,
    arriving: Option<(Vec<String>, Slot)>,
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

fn thumbnail(image: &Image, room: Rect, palette: Palette, items: &mut Vec<SceneItem>) {
    let (width, height) = (image.width() as f32, image.height() as f32);
    let scale = (room.width / width).min(room.height / height);
    let (shown_width, shown_height) = ((width * scale).round(), (height * scale).round());
    let shown = Rect::new(
        (room.x + (room.width - shown_width) / 2.0).round(),
        (room.y + room.height - shown_height).round(),
        shown_width,
        shown_height,
    );
    let shape = Shape::rounded(shown, THUMBNAIL_RADIUS);
    items.extend(SceneItem::shadows(
        shape,
        Elevation::Overlay,
        palette.scheme,
        true,
    ));
    items.push(SceneItem::Clip {
        shape,
        items: vec![SceneItem::Image {
            rect: shown,
            image: image.clone(),
        }],
    });
    items.push(SceneItem::Stroke {
        shape: Shape::rounded(shown.inset(0.5), THUMBNAIL_RADIUS - 0.5),
        width: 1.0,
        paint: palette.edge.with_opacity(THUMBNAIL_EDGE).into(),
    });
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
    pub fn new(appearance: Appearance, layouts: Layouts, text: TextSystem) -> Self {
        let layout = FreeLayout::new();
        let layout_id = layout.id();
        let mut ui = Ui::new(Box::new(layout), Context::new(text, appearance.palette()));
        ui.set_appearance(appearance);
        Self {
            desktop: Desktop::new(Grid::new(Bounds::new(0.0, 0.0, 1.0, 1.0))),
            layouts,
            layouts_changed: false,
            icons: HashMap::new(),
            labels: HashMap::new(),
            motions: HashMap::new(),
            backdrop: Backdrop::default(),
            wallpaper_unavailable: false,
            ui,
            layout: layout_id,
            editor: None,
            editing: false,
            folder: PathBuf::new(),
            drop: None,
            offered: Vec::new(),
            arriving: None,
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

    pub fn set_items(&mut self, items: Vec<(Item, Option<Artwork>)>, now: Instant) {
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
        if let Some((names, slot)) = self.arriving.take() {
            let (arrived, waiting): (Vec<String>, Vec<String>) = names
                .into_iter()
                .partition(|name| self.desktop.item(&self.folder.join(name)).is_some());
            let paths: Vec<PathBuf> = arrived.iter().map(|name| self.folder.join(name)).collect();
            self.desktop.place_at(&paths, slot);
            if !waiting.is_empty() {
                self.arriving = Some((waiting, slot));
            }
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

    pub fn wallpaper_unavailable(&mut self) {
        self.wallpaper_unavailable = true;
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

    pub fn deadline(&self) -> Option<Instant> {
        let typing = self.typed_at.map(|at| at + TYPE_SELECT_PAUSE);
        [typing, self.spring_due()].into_iter().flatten().min()
    }

    pub fn is_animating(&self) -> bool {
        self.animating
    }

    fn spring_due(&self) -> Option<Instant> {
        self.drop
            .as_ref()
            .filter(|hover| {
                !hover.sprung && hover.action.is_some() && matches!(hover.target, Target::Folder(_))
            })
            .map(|hover| hover.since + SPRING_DELAY)
    }

    pub fn tick(&mut self, now: Instant) {
        if self.spring_due().is_some_and(|due| due <= now)
            && let Some(hover) = &mut self.drop
            && let Target::Folder(folder) = &hover.target
        {
            hover.sprung = true;
            self.requests.push(Request::Open(vec![folder.clone()]));
        }
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
        let mut field = TextField::new("", &name, |_| {}).in_place(LABEL_SIZE);
        field.select(0..stem);
        let at = self.editor_bounds(path).map(rect).unwrap_or_default();
        self.remove_editor();
        let Some(layout) = self.ui.widget_as::<FreeLayout>(self.layout) else {
            return;
        };
        let field_id = layout.insert(field, at);
        self.editor = Some(field_id);
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
        self.remove_editor();
    }

    fn remove_editor(&mut self) {
        if let Some((editor, layout)) = self
            .editor
            .take()
            .zip(self.ui.widget_as::<FreeLayout>(self.layout))
        {
            layout.remove(editor);
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
        if let Some(request) = self.ui.take_clipboard_request() {
            self.requests.push(Request::Clipboard(request));
        }
        handled
    }

    fn without_volumes(&self, paths: Vec<PathBuf>) -> Vec<PathBuf> {
        paths
            .into_iter()
            .filter(|path| {
                self.desktop
                    .item(path)
                    .is_some_and(|item| item.kind != Kind::Volume)
            })
            .collect()
    }

    fn on_files(&mut self, paths: Vec<PathBuf>, request: fn(Vec<PathBuf>) -> Request) {
        let files = self.without_volumes(paths);
        if !files.is_empty() {
            self.requests.push(request(files));
        }
    }

    fn ask_to_delete(&mut self, paths: Vec<PathBuf>) {
        let deletable = self.without_volumes(paths);
        if deletable.is_empty() {
            return;
        }
        self.cancel_rename();
        self.requests.push(Request::ConfirmDelete(deletable));
        self.gesture = Gesture::Idle;
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
        match event {
            Event::PointerDown(pointer) => self.pointer_down(event, pointer),
            Event::PointerMove(_) | Event::PointerUp(_) if self.editing => {
                if matches!(event, Event::PointerUp(_)) {
                    self.editing = false;
                }
                self.edit(event)
            }
            Event::PointerMove(pointer) => self.pointer_move(pointer),
            Event::PointerUp(pointer) => self.pointer_up(pointer),
            Event::ModifiersChanged(modifiers) => {
                self.modifiers = *modifiers;
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
                let paths = self.desktop.selected();
                self.gesture = Gesture::Drag {
                    from: *from,
                    paths: paths.clone(),
                };
                self.requests.push(Request::StartDrag(paths));
                true
            }
            Gesture::Band { from, to, keep } => {
                *to = at;
                let area = Bounds::spanning((from.x, from.y), (at.x, at.y));
                let keep = keep.clone();
                self.desktop.select_in(area, &keep);
                true
            }
            Gesture::Drag { .. } => false,
        }
    }

    fn pointer_up(&mut self, pointer: &Pointer) -> bool {
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
            Gesture::Drag { .. } => false,
        }
    }

    pub fn set_folder(&mut self, folder: PathBuf) {
        self.folder = folder;
    }

    pub fn drag_icon(&mut self) -> DragIcon {
        let Gesture::Drag { from, paths } = self.gesture.clone() else {
            return DragIcon {
                scene: Scene {
                    width: 1,
                    height: 1,
                    background: Color::TRANSPARENT,
                    items: Vec::new(),
                },
                hotspot: Point::new(0.0, 0.0),
            };
        };
        let palette = self.appearance.palette();
        let grid = self.desktop.grid();
        let cells: Vec<(Item, Bounds)> = paths
            .iter()
            .filter_map(|path| {
                let item = self.desktop.item(path)?.clone();
                let cell = grid.cell(self.desktop.slot_of(path)?);
                Some((item, cell))
            })
            .collect();
        let badge = (cells.len() > 1).then(|| {
            let count = self.ui.context().text.paragraph(
                &cells.len().to_string(),
                TextStyle::new(BADGE_TEXT, palette.on_danger)
                    .weight(TextStyle::BOLD)
                    .tabular(),
                None,
                TextAlign::Start,
            );
            let width = (count.width() + 2.0 * BADGE_PADDING)
                .max(BADGE_HEIGHT)
                .ceil();
            let rect = Rect::new(
                (from.x + BADGE_OFFSET.0 - width / 2.0).round(),
                (from.y + BADGE_OFFSET.1 - BADGE_HEIGHT / 2.0).round(),
                width,
                BADGE_HEIGHT,
            );
            (count, rect)
        });
        let reach = cells.iter().map(|(_, cell)| *cell).chain(
            badge
                .as_ref()
                .map(|(_, rect)| Bounds::new(rect.x, rect.y, rect.width, rect.height)),
        );
        let (left, top, right, bottom) = reach.fold(
            (from.x, from.y, from.x, from.y),
            |(left, top, right, bottom), cell| {
                (
                    left.min(cell.x),
                    top.min(cell.y),
                    right.max(cell.x + cell.width),
                    bottom.max(cell.y + cell.height),
                )
            },
        );
        let mut items = Vec::new();
        for (item, cell) in cells {
            self.paint_item(&item, cell, 1.0, palette, &mut items);
        }
        let mut drawn = vec![SceneItem::Opacity {
            alpha: DRAGGED,
            items,
        }];
        if let Some((count, rect)) = badge {
            drawn.push(SceneItem::Fill {
                shape: Shape::rounded(rect, BADGE_HEIGHT / 2.0),
                paint: palette.danger.into(),
            });
            drawn.push(SceneItem::Text {
                origin: Point::new(
                    (rect.x + (rect.width - count.width()) / 2.0).round(),
                    (rect.y + (rect.height - count.height()) / 2.0).round(),
                ),
                paragraph: count,
            });
        }
        DragIcon {
            scene: Scene {
                width: (right - left).ceil().max(1.0) as u32,
                height: (bottom - top).ceil().max(1.0) as u32,
                background: Color::TRANSPARENT,
                items: vec![SceneItem::Translate {
                    offset: Point::new(-left, -top),
                    items: drawn,
                }],
            },
            hotspot: Point::new(from.x - left, from.y - top),
        }
    }

    fn target(&self, at: Point) -> Target {
        self.desktop
            .hit(at.x, at.y)
            .filter(|item| matches!(item.kind, Kind::Folder | Kind::Volume))
            .filter(|item| !self.offered.contains(&item.path))
            .map_or(Target::Desktop, |item| Target::Folder(item.path.clone()))
    }

    pub fn drag_entered(&mut self, at: Point, uris: &[String]) -> Option<DragAction> {
        self.offered = uris
            .iter()
            .filter_map(|uri| path_from_file_uri(uri))
            .collect();
        if self.offered.is_empty() {
            self.drop = None;
            return None;
        }
        self.drag_moved(at)
    }

    pub fn drag_moved(&mut self, at: Point) -> Option<DragAction> {
        let target = self.target(at);
        let action = match &target {
            Target::Desktop => Some(DragAction::Move),
            Target::Folder(folder) => {
                let reference = self.offered.first().unwrap_or(&self.folder);
                match same_volume(reference, folder) {
                    true => Some(DragAction::Move),
                    false => Some(DragAction::Copy),
                }
            }
        };
        let kept = self
            .drop
            .take()
            .filter(|hover| hover.target == target)
            .map(|hover| (hover.since, hover.sprung));
        let (since, sprung) = kept.unwrap_or((Instant::now(), false));
        self.drop = Some(Hover {
            target,
            action,
            since,
            sprung,
        });
        action
    }

    pub fn drag_left(&mut self) {
        self.drop = None;
        self.offered.clear();
    }

    pub fn drag_finished(&mut self) {
        self.drop = None;
        self.offered.clear();
        if matches!(self.gesture, Gesture::Drag { .. }) {
            self.gesture = Gesture::Idle;
        }
    }

    pub fn dropped(
        &mut self,
        at: Point,
        uris: &[String],
        action: Option<DragAction>,
        now: Instant,
    ) -> bool {
        self.drop = None;
        self.offered.clear();
        let sources: Vec<PathBuf> = uris
            .iter()
            .filter_map(|uri| path_from_file_uri(uri))
            .collect();
        if sources.is_empty() {
            return false;
        }
        let copy = action == Some(DragAction::Copy);
        let own = match &self.gesture {
            Gesture::Drag { from, paths } if *paths == sources => Some(*from),
            _ => None,
        };
        match (self.target(at), own) {
            (Target::Folder(folder), _) => {
                let sources: Vec<PathBuf> = sources
                    .into_iter()
                    .filter(|source| {
                        copy || self
                            .desktop
                            .item(source)
                            .is_none_or(|item| item.kind != Kind::Volume)
                    })
                    .collect();
                if !sources.is_empty() {
                    self.requests.push(Request::Transfer {
                        sources,
                        folder,
                        copy,
                    });
                }
            }
            (Target::Desktop, Some(_)) if copy => self.on_files(sources, Request::Duplicate),
            (Target::Desktop, Some(from)) => {
                let grid = self.desktop.grid();
                let (start, end) = (grid.slot_at(from.x, from.y), grid.slot_at(at.x, at.y));
                let by = (
                    i64::from(end.column) - i64::from(start.column),
                    i64::from(end.row) - i64::from(start.row),
                );
                self.desktop.move_items(&sources, by);
                self.retarget(now);
                self.remember();
            }
            (Target::Desktop, None) => {
                let here = sources
                    .iter()
                    .all(|source| source.parent() == Some(self.folder.as_path()));
                if here {
                    return false;
                }
                let names = sources
                    .iter()
                    .filter_map(|source| source.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .collect();
                self.arriving = Some((names, self.desktop.grid().slot_at(at.x, at.y)));
                let folder = self.folder.clone();
                self.requests.push(Request::Transfer {
                    sources,
                    folder,
                    copy,
                });
            }
        }
        true
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
                self.on_files(selected, Request::Trash);
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
                'd' if !selected.is_empty() => self.on_files(selected, Request::Duplicate),
                'c' if !selected.is_empty() => self.requests.push(Request::Copy(selected)),
                'x' if !selected.is_empty() => self.on_files(selected, Request::Cut),
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
            Command::Duplicate => {
                self.on_files(selected, Request::Duplicate);
                return;
            }
            Command::Copy => Request::Copy(selected),
            Command::Cut => {
                self.on_files(selected, Request::Cut);
                return;
            }
            Command::Paste => Request::Paste { moves: false },
            Command::Trash => {
                self.on_files(selected, Request::Trash);
                return;
            }
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
        let selected = selected.then_some(self.focused);
        let on_light = selected.is_none()
            && self
                .backdrop
                .is_light(bounds, self.scale)
                .unwrap_or(self.appearance.scheme == Scheme::Light);
        if let Some(label) = self.labels.get(&item.path)
            && label.name == item.name
            && label.on_light == on_light
            && label.selected == selected
        {
            return label.clone();
        }
        let palette = self.appearance.palette();
        let (color, shadow) = match selected {
            Some(true) => (palette.on_accent, None),
            Some(false) => (palette.ink, None),
            None => {
                let wallpaper = Palette::for_scheme(if on_light {
                    Scheme::Light
                } else {
                    Scheme::Dark
                });
                let halo = match on_light {
                    true => wallpaper.sheet,
                    false => Color::BLACK,
                };
                (wallpaper.ink, Some(halo.with_opacity(LABEL_HALO)))
            }
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
            drawn.push(fill(backdrop, Radius::L, palette.scrim));
        }
        let grow = 0.85 + 0.15 * shown;
        let scaled = Bounds::new(
            icon.x + icon.width * (1.0 - grow) / 2.0,
            icon.y + icon.height * (1.0 - grow) / 2.0,
            icon.width * grow,
            icon.height * grow,
        );
        match self.icons.get(&item.path).cloned().flatten() {
            Some(Artwork::Icon(image)) => drawn.push(SceneItem::Image {
                rect: rect(scaled),
                image,
            }),
            Some(Artwork::Thumbnail(image)) => {
                thumbnail(
                    &image,
                    rect(scaled).inset(THUMBNAIL_INSET),
                    palette,
                    &mut drawn,
                );
            }
            None => drawn.push(fill(scaled.inset(8.0), Radius::M, palette.overlay)),
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
                    palette.accent
                } else {
                    palette.well
                };
                drawn.push(fill(pill, Radius::XS, color));
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

    fn place_editor(&mut self) {
        let Some(rename) = &self.rename else {
            return;
        };
        let Some(field) = self.editor_bounds(&rename.path) else {
            return;
        };
        if let Some((editor, layout)) = self
            .editor
            .zip(self.ui.widget_as::<FreeLayout>(self.layout))
        {
            layout.set_frame(editor, rect(field));
        }
    }

    pub fn scene(&mut self, now: Instant) -> Scene {
        let palette = self.appearance.palette();
        let (width, height) = self.size;
        let mut items = Vec::new();
        self.backdrop
            .paint(Rect::new(0.0, 0.0, width, height), now, &mut items);
        let mut animating = self.backdrop.is_fading(now);
        let dragging = match &self.gesture {
            Gesture::Drag { paths, .. } => paths.clone(),
            _ => Vec::new(),
        };
        let target = match &self.drop {
            Some(Hover {
                target: Target::Folder(folder),
                action: Some(_),
                ..
            }) => Some(folder.clone()),
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
        for (item, cell, shown) in &entries {
            if target.as_ref() == Some(&item.path) {
                let icon = icon_bounds(*cell);
                let well = Bounds::new(
                    icon.x - 5.0,
                    icon.y - 5.0,
                    icon.width + 10.0,
                    icon.height + 10.0,
                );
                items.push(fill(
                    well,
                    Radius::L,
                    palette.accent.with_opacity(DROP_TARGET),
                ));
            }
            match dragging.contains(&item.path) {
                true => {
                    let mut lifted = Vec::new();
                    self.paint_item(item, *cell, *shown, palette, &mut lifted);
                    items.push(SceneItem::Opacity {
                        alpha: LIFTED,
                        items: lifted,
                    });
                }
                false => self.paint_item(item, *cell, *shown, palette, &mut items),
            }
        }
        if let Gesture::Band { from, to, .. } = &self.gesture {
            let band = Bounds::spanning((from.x, from.y), (to.x, to.y));
            items.push(fill(
                band,
                Radius::XS,
                palette.accent.with_opacity(BAND_FILL),
            ));
            items.push(SceneItem::Stroke {
                shape: Shape::rounded(rect(band).inset(0.5), Radius::XS - 0.5),
                width: 1.0,
                paint: palette.accent_text.into(),
            });
        }
        self.place_editor();
        items.extend(
            self.ui
                .scene(width.round() as u32, height.round() as u32)
                .items,
        );
        self.animating = animating || self.ui.is_animating();
        Scene {
            width: width.round() as u32,
            height: height.round() as u32,
            background: match self.backdrop.key().is_some() || self.wallpaper_unavailable {
                true => palette.chassis,
                false => Color::TRANSPARENT,
            },
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
                toolkit_name: Some("bande-toolkit".to_owned()),
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
