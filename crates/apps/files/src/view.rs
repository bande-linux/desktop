use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use unbound_fs::{
    Action, Answer, Change, Entry, Finished, FolderWatch, Operations, Purpose, RenameError,
    Resolution, Task, TrashItem,
};
use unbound_preview::Preview;
use unbound_toolkit::ui::{
    Context, Cursor, Event, FreeLayout, Key, Menu, Modifiers, Pointer, PointerButton, Scroll,
    ScrollState, Ui, Widget, WidgetId,
};
use unbound_toolkit::{Image, Item, Palette, Point, Rect, TextSystem};
use unbound_volumes::mounted_volumes;
use unbound_xdg::{Apps, DesktopEntry, UserDir, path_from_file_uri, user_dir};

use crate::L10N;
use crate::browser::{Browser, Location};
use crate::controls::{Press, Signals};
use crate::icons::Icons;
use crate::loader::{Loaded, Loader};
use crate::sidebar::{Place, PlaceKind, Row, with_volumes};

pub const COLUMN_WIDTH: f32 = 240.0;
pub const ROW_HEIGHT: f32 = 24.0;
pub const SIDEBAR_WIDTH: f32 = 180.0;
pub const INSPECTOR_WIDTH: f32 = 260.0;
pub const PATH_BAR_HEIGHT: f32 = 28.0;
pub const PROGRESS_HEIGHT: f32 = 48.0;
pub const TOP_INSET: f32 = 6.0;
pub const HEADER_HEIGHT: f32 = 26.0;
pub const SEARCH_HEIGHT: f32 = 36.0;
pub(crate) const SLOW_CLICK_RENAME: Duration = Duration::from_millis(600);
const TYPE_SELECT_PAUSE: Duration = Duration::from_millis(1000);
const NOTICE_TIME: Duration = Duration::from_secs(5);

pub struct QuickLook {
    pub entry: Entry,
    pub preview: Option<Preview<Image>>,
}

pub struct Info {
    pub entries: Vec<Entry>,
    pub mime: String,
    pub kind: String,
    pub apps: Vec<DesktopEntry>,
    pub default: Option<String>,
    pub chosen: Option<usize>,
    pub folder_size: Option<(u64, u64)>,
    pub preview: Option<Preview<Image>>,
}

pub struct Search {
    pub column: usize,
    pub(crate) field: WidgetId,
}

pub struct Rename {
    pub path: PathBuf,
    pub(crate) field: WidgetId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sheet {
    Conflict {
        apply_to_all: bool,
    },
    EmptyTrash,
    DeleteImmediately {
        paths: Vec<PathBuf>,
        items: Vec<TrashItem>,
    },
    GoTo(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    Place(usize),
    Eject(usize),
    Row { column: usize, row: usize },
    Column(usize),
    Inspector,
    Segment(PathBuf),
    CancelJob(usize),
    ChooseApp(usize),
    OpenWithChosen,
    UseForAll,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Open {
        app: Box<DesktopEntry>,
        files: Vec<PathBuf>,
    },
    CopyText(String),
    Paste,
    CopyFiles {
        paths: Vec<PathBuf>,
        cut: bool,
    },
    StartDrag {
        paths: Vec<PathBuf>,
        grab: Point,
    },
    PasteFiles,
    ShowMenu {
        menu: Menu,
        at: Point,
    },
    RemoveBookmark(PathBuf),
    Eject(PathBuf),
}

pub struct FilesView {
    pub(crate) browser: Browser,
    pub(crate) sidebar: Vec<Row>,
    pub(crate) apps: Apps,
    pub(crate) icons: Icons,
    pub(crate) palette: Palette,
    pub(crate) ui: Ui,
    pub(crate) host: WidgetId,
    pub(crate) table: WidgetId,
    pub(crate) signals: Signals,
    pub(crate) table_revision: Option<u64>,
    pub(crate) table_keys: HashMap<String, usize>,
    pub(crate) table_revealed: Option<usize>,
    pub(crate) press: Option<Press>,
    pub(crate) drag_sources: Vec<PathBuf>,
    pub(crate) drop: Option<crate::drag::Hover>,
    scrolls: Vec<(Location, ScrollState)>,
    pub(crate) sheet_cancel: Option<WidgetId>,
    pub(crate) size: (f32, f32),
    pub(crate) quick_look: Option<QuickLook>,
    pub(crate) inspector: bool,
    pub(crate) info: Option<Info>,
    pub(crate) rename: Option<Rename>,
    pub(crate) sheet: Option<Sheet>,
    pub(crate) notice: Option<(String, Instant)>,
    pub(crate) hits: Vec<(Rect, Hit)>,
    pub(crate) operations: Operations,
    pub(crate) trash_full: bool,
    pub(crate) hover: Option<Hit>,
    pub(crate) search: Option<Search>,
    pub(crate) menu_place: Option<usize>,
    media: Vec<PathBuf>,
    favourites: Vec<Row>,
    watch: FolderWatch,
    loader: Loader,
    pub(crate) requests: Vec<Request>,
    pub(crate) pending_rename: Option<(Instant, PathBuf)>,
    pointer: Point,
    typed: (String, Option<Instant>),
    pub(crate) paste_moves: Option<bool>,
    pub(crate) clipboard_files: bool,
}

pub(crate) fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

fn user_folder(dir: UserDir) -> PathBuf {
    let home = home();
    let user_dirs = unbound_xdg::config_home()
        .and_then(|config| std::fs::read_to_string(config.join("user-dirs.dirs")).ok());
    user_dir(&home, user_dirs.as_deref(), dir)
}

pub(crate) fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x
        && point.y >= rect.y
        && point.x < rect.x + rect.width
        && point.y < rect.y + rect.height
}

fn pasted_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter_map(|line| {
            path_from_file_uri(line)
                .or_else(|| Path::new(line).is_absolute().then(|| PathBuf::from(line)))
        })
        .filter(|path| path.symlink_metadata().is_ok())
        .collect()
}

impl FilesView {
    pub fn new(
        browser: Browser,
        sidebar: Vec<Row>,
        apps: Apps,
        palette: Palette,
        text: TextSystem,
    ) -> io::Result<Self> {
        let icons = Icons::new(apps.data_dirs().to_vec());
        let operations = Operations::new(browser.trash().clone());
        let signals: Signals = Rc::new(RefCell::new(Vec::new()));
        let mut host = FreeLayout::new();
        let table = crate::controls::table(&signals);
        let table_id = Widget::id(&table);
        host.insert(table, Rect::default());
        let host_id = Widget::id(&host);
        let mut view = Self {
            browser,
            sidebar,
            apps,
            icons,
            palette,
            ui: Ui::new(Box::new(host), Context::new(text, palette)),
            host: host_id,
            table: table_id,
            signals,
            table_revision: None,
            table_keys: HashMap::new(),
            table_revealed: None,
            press: None,
            drag_sources: Vec::new(),
            drop: None,
            scrolls: Vec::new(),
            sheet_cancel: None,
            size: (0.0, 0.0),
            quick_look: None,
            inspector: false,
            info: None,
            rename: None,
            sheet: None,
            notice: None,
            hits: Vec::new(),
            operations,
            trash_full: false,
            hover: None,
            search: None,
            menu_place: None,
            media: Vec::new(),
            favourites: Vec::new(),
            watch: FolderWatch::new()?,
            loader: Loader::new()?,
            requests: Vec::new(),
            pending_rename: None,
            pointer: Point::default(),
            typed: (String::new(), None),
            paste_moves: None,
            clipboard_files: false,
        };
        view.favourites = view.sidebar.clone();
        view.watch.watch(&view.watched());
        view.trash_full = !view.browser.trash().is_empty();
        view.focus_content();
        Ok(view)
    }

    pub fn set_sidebar(&mut self, sidebar: Vec<Row>) {
        self.sidebar = with_volumes(&sidebar, &self.volumes());
        self.favourites = sidebar;
        self.menu_place = None;
    }

    pub fn set_media(&mut self, roots: Vec<PathBuf>) {
        self.media = roots;
        self.refresh_volumes();
        self.watch.watch(&self.watched());
    }

    fn volumes(&self) -> Vec<PathBuf> {
        self.sidebar
            .iter()
            .filter_map(|row| match row {
                Row::Place(Place {
                    kind: PlaceKind::Volume,
                    location: Location::Folder(path),
                    ..
                }) => Some(path.clone()),
                _ => None,
            })
            .collect()
    }

    fn refresh_volumes(&mut self) {
        let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
        let volumes = mounted_volumes(&mountinfo, &self.media);
        let previous = self.volumes();
        if volumes == previous {
            return;
        }
        self.sidebar = with_volumes(&self.favourites, &volumes);
        self.menu_place = None;
        let gone = previous.iter().any(|volume| {
            !volumes.contains(volume)
                && matches!(self.browser.root(), Location::Folder(path) if path.starts_with(volume))
        });
        if gone {
            self.go(Location::Folder(home()));
        }
    }

    fn watched(&self) -> Vec<PathBuf> {
        let mut folders = self.browser.watched();
        folders.extend(
            self.media
                .iter()
                .flat_map(|root| [Some(root.as_path()), root.parent()])
                .flatten()
                .filter(|folder| folder.is_dir())
                .map(Path::to_path_buf),
        );
        folders
    }

    pub(crate) fn eject_current(&mut self) {
        let Some(folder) = self.browser.current_folder() else {
            return;
        };
        if let Some(volume) = self
            .volumes()
            .into_iter()
            .find(|volume| folder.starts_with(volume))
        {
            self.requests.push(Request::Eject(volume));
        }
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
        self.ui.context().palette = palette;
    }

    pub(crate) fn text(&mut self) -> &mut TextSystem {
        &mut self.ui.context().text
    }

    pub fn cursor(&self) -> Cursor {
        self.ui.cursor()
    }

    pub fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    pub fn browser(&self) -> &Browser {
        &self.browser
    }

    pub fn quick_look(&self) -> Option<&QuickLook> {
        self.quick_look.as_ref()
    }

    pub fn info(&self) -> Option<&Info> {
        self.info.as_ref()
    }

    pub fn renaming(&self) -> Option<&Path> {
        self.rename.as_ref().map(|rename| rename.path.as_path())
    }

    pub fn sheet(&self) -> Option<&Sheet> {
        self.sheet.as_ref()
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_ref().map(|(text, _)| text.as_str())
    }

    pub fn operations(&self) -> &Operations {
        &self.operations
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn title(&self) -> String {
        match self.browser.current_location() {
            Location::Trash => L10N.tr("trash"),
            Location::Folder(folder) => folder.file_name().map_or_else(
                || folder.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            ),
        }
    }

    pub fn fds(&self) -> Vec<BorrowedFd<'_>> {
        let mut fds = vec![self.watch.fd(), self.loader.fd()];
        fds.extend(self.operations.fds());
        fds
    }

    pub fn source_ready(&mut self, index: usize) -> bool {
        match index {
            0 => {
                self.watch.read();
                false
            }
            1 => self.loaded(),
            job => match self.operations.poll(job - 2) {
                Some(finished) => {
                    self.finished(finished);
                    true
                }
                None => {
                    if self.operations.conflict().is_some() && self.sheet.is_none() {
                        self.present_conflict();
                    }
                    true
                }
            },
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        [
            self.watch.deadline(),
            self.operations.deadline(),
            self.notice.as_ref().map(|(_, shown)| *shown + NOTICE_TIME),
            self.pending_rename.as_ref().map(|(due, _)| *due),
            self.drop.as_ref().and_then(crate::drag::Hover::due),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        let changed = self.watch.take_changed(now);
        if !changed.is_empty() {
            self.refresh(&changed);
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, shown)| *shown + NOTICE_TIME <= now)
        {
            self.notice = None;
        }
        self.spring(now);
        if let Some((due, path)) = self.pending_rename.take() {
            if due > now {
                self.pending_rename = Some((due, path));
            } else if self
                .browser
                .selected()
                .is_some_and(|entry| entry.path == path)
            {
                self.start_rename();
            }
        }
        true
    }

    pub fn refresh(&mut self, changed: &[PathBuf]) {
        if changed
            .iter()
            .any(|path| self.media.iter().any(|root| root.starts_with(path)))
        {
            self.refresh_volumes();
        }
        let editing = self.rename.as_ref().map(|rename| rename.path.clone());
        self.trash_full = !self.browser.trash().is_empty();
        self.browser.refresh(changed);
        if let Some(path) = editing
            && path.symlink_metadata().is_err()
        {
            self.cancel_rename();
        }
        self.selection_changed();
    }

    fn show_notice(&mut self, text: String) {
        self.notice = Some((text, Instant::now()));
    }

    fn loaded(&mut self) -> bool {
        let mut changed = false;
        for loaded in self.loader.take() {
            match loaded {
                Loaded::Preview(path, preview) => {
                    if let Some(quick_look) = &mut self.quick_look
                        && quick_look.entry.path == path
                    {
                        quick_look.preview = Some(preview.clone());
                        changed = true;
                    }
                    if let Some(info) = &mut self.info
                        && info.entries.len() == 1
                        && info.entries[0].path == path
                    {
                        info.preview = Some(preview);
                        changed = true;
                    }
                }
                Loaded::FolderSize {
                    folder,
                    items,
                    bytes,
                } => {
                    if let Some(info) = &mut self.info
                        && info.entries.len() == 1
                        && info.entries[0].path == folder
                    {
                        info.folder_size = Some((items, bytes));
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    pub(crate) fn selection_changed(&mut self) {
        if self
            .search
            .as_ref()
            .is_some_and(|search| search.column >= self.browser.columns().len())
        {
            self.search = None;
        }
        self.watch.watch(&self.watched());
        self.loader.forget_pending();
        let entry = self.browser.selected().cloned();
        if self.quick_look.is_some() {
            self.quick_look = entry.clone().map(|entry| QuickLook {
                entry,
                preview: None,
            });
        }
        if self.inspector {
            self.update_info();
        } else {
            self.info = None;
        }
        if let Some(entry) = entry
            && (self.quick_look.is_some() || self.inspector)
        {
            self.loader.preview(entry.clone());
            if self.inspector && entry.is_folder() {
                self.loader.folder_size(entry.path);
            }
        }
        self.keep_selection_visible();
        self.sync_table();
    }

    pub(crate) fn update_info(&mut self) {
        let entries = self
            .browser
            .selection()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let previous = self.info.take();
        let same = previous
            .as_ref()
            .is_some_and(|info| info.entries == entries);
        if same {
            self.info = previous;
            return;
        }
        let mime = match entries.as_slice() {
            [entry] => self.apps.mime_for(&entry.path),
            _ => String::new(),
        };
        let apps = match entries.as_slice() {
            [entry] if !entry.is_folder() => self
                .apps
                .apps_for(&mime)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let default = self.apps.default_for(&mime).map(|app| app.id.clone());
        let chosen = default
            .as_ref()
            .and_then(|id| apps.iter().position(|app| app.id == *id))
            .or((!apps.is_empty()).then_some(0));
        self.info = Some(Info {
            kind: if mime.is_empty() {
                String::new()
            } else {
                self.apps.kind(&mime)
            },
            entries,
            mime,
            apps,
            default,
            chosen,
            folder_size: None,
            preview: None,
        });
    }

    pub fn content_frame(&self) -> Rect {
        let inspector = if self.inspector { INSPECTOR_WIDTH } else { 0.0 };
        let progress = self.visible_jobs() as f32 * PROGRESS_HEIGHT;
        let search = if self.search.is_some() {
            SEARCH_HEIGHT
        } else {
            0.0
        };
        Rect::new(
            SIDEBAR_WIDTH,
            search,
            (self.size.0 - SIDEBAR_WIDTH - inspector).max(COLUMN_WIDTH),
            (self.size.1 - PATH_BAR_HEIGHT - progress - search).max(ROW_HEIGHT),
        )
    }

    pub(crate) fn visible_jobs(&self) -> usize {
        let now = Instant::now();
        self.operations
            .running()
            .iter()
            .filter(|running| running.visible(now))
            .count()
    }

    pub(crate) fn first_visible_column(&self) -> usize {
        let columns = self.browser.columns().len();
        let last = (self.browser.active() + 1).min(columns - 1);
        let fit = ((self.content_frame().width / COLUMN_WIDTH).floor() as usize).max(1);
        (last + 1).saturating_sub(fit)
    }

    fn rows_height(&self) -> f32 {
        self.content_frame().height - self.rows_top()
    }

    pub(crate) fn rows_top(&self) -> f32 {
        if self.browser.single() {
            HEADER_HEIGHT
        } else {
            TOP_INSET
        }
    }

    fn fit_scrolls(&mut self) {
        let viewport = self.rows_height();
        let columns = self.browser.columns();
        self.scrolls.truncate(columns.len());
        for (index, column) in columns.iter().enumerate() {
            let content = column.entries.len() as f32 * ROW_HEIGHT;
            match self.scrolls.get_mut(index) {
                Some((location, _)) if *location == column.location => {}
                Some(slot) => *slot = (column.location.clone(), ScrollState::default()),
                None => self
                    .scrolls
                    .push((column.location.clone(), ScrollState::default())),
            }
            self.scrolls[index].1.set_extent(content, viewport);
        }
    }

    pub(crate) fn column_offset(&mut self, index: usize) -> f32 {
        self.fit_scrolls();
        let cx = self.ui.context();
        self.scrolls
            .get(index)
            .map_or(0.0, |(_, scroll)| scroll.offset(cx))
    }

    pub(crate) fn paint_column_scrollbar(
        &mut self,
        index: usize,
        frame: Rect,
        items: &mut Vec<Item>,
    ) {
        let cx = self.ui.context();
        if let Some((_, scroll)) = self.scrolls.get(index) {
            scroll.paint_scrollbar(cx, frame, items);
        }
    }

    fn keep_selection_visible(&mut self) {
        self.fit_scrolls();
        let rows = self
            .browser
            .columns()
            .iter()
            .map(|column| column.selected)
            .collect::<Vec<_>>();
        let cx = self.ui.context();
        for ((_, scroll), row) in self.scrolls.iter_mut().zip(rows) {
            if let Some(row) = row {
                let top = row as f32 * ROW_HEIGHT;
                scroll.reveal(cx, top, top + ROW_HEIGHT);
            }
        }
    }

    fn scroll(&mut self, scroll: &Scroll) -> bool {
        let content = self.content_frame();
        if self.browser.single() || !contains(content, scroll.position) {
            return false;
        }
        let column =
            self.first_visible_column() + ((scroll.position.x - content.x) / COLUMN_WIDTH) as usize;
        self.fit_scrolls();
        let cx = self.ui.context();
        self.scrolls
            .get_mut(column)
            .is_some_and(|(_, state)| state.scroll(cx, scroll))
    }

    pub(crate) fn hit(&self, at: Point) -> Option<Hit> {
        self.hits
            .iter()
            .rev()
            .find(|(rect, _)| contains(*rect, at))
            .map(|(_, hit)| hit.clone())
    }

    fn click(&mut self, pointer: &Pointer) -> bool {
        match pointer.button {
            PointerButton::Back => {
                self.browser.go_back();
                self.selection_changed();
                return true;
            }
            PointerButton::Forward => {
                self.browser.go_forward();
                self.selection_changed();
                return true;
            }
            _ => {}
        }
        let Some(hit) = self.hit(pointer.position) else {
            return false;
        };
        if pointer.button == PointerButton::Secondary
            || (pointer.is_primary() && pointer.modifiers.control)
        {
            return self.context_click(hit, pointer.position);
        }
        if !pointer.is_primary() {
            return false;
        }
        self.pending_rename = None;
        match hit {
            Hit::Row { column, row } => {
                let modifiers = pointer.modifiers;
                let plain = !modifiers.logo && !modifiers.shift;
                let shown = &self.browser.columns()[column];
                let path = shown.entries[row].path.clone();
                let already = self
                    .browser
                    .selected()
                    .is_some_and(|entry| entry.path == path)
                    && self.browser.active() == column;
                let keep = plain
                    && pointer.count == 1
                    && column == self.browser.active()
                    && shown.is_marked(row)
                    && shown.marked.len() > 1;
                self.commit_rename();
                if plain && pointer.count == 1 {
                    self.press = Some(Press::Item {
                        at: pointer.position,
                        collapse: keep.then(|| path.clone()),
                    });
                }
                if keep {
                } else if modifiers.logo {
                    self.browser.toggle(column, row);
                } else if modifiers.shift {
                    self.browser.extend_to(column, row);
                } else {
                    self.browser.select(column, row);
                }
                self.selection_changed();
                if pointer.count >= 2 && !modifiers.logo && !modifiers.shift {
                    self.open();
                } else if already && pointer.count == 1 && !modifiers.logo && !modifiers.shift {
                    self.pending_rename = Some((Instant::now() + SLOW_CLICK_RENAME, path));
                }
            }
            Hit::Column(column) => {
                self.commit_rename();
                self.browser.focus_column(column);
                self.selection_changed();
            }
            Hit::Place(index) => {
                if let Some(Row::Place(place)) = self.sidebar.get(index) {
                    let location = place.location.clone();
                    self.go(location);
                }
            }
            Hit::Segment(path) => {
                if path.is_dir() {
                    self.go(Location::Folder(path));
                } else {
                    self.browser.reveal(&path);
                    self.selection_changed();
                }
            }
            Hit::Eject(index) => {
                if let Some(Row::Place(place)) = self.sidebar.get(index)
                    && let Location::Folder(path) = &place.location
                {
                    self.requests.push(Request::Eject(path.clone()));
                }
            }
            Hit::CancelJob(index) => self.operations.cancel(index),
            Hit::ChooseApp(index) => {
                if let Some(info) = &mut self.info {
                    info.chosen = Some(index);
                }
            }
            Hit::OpenWithChosen => self.open_with_chosen(),
            Hit::UseForAll => self.use_chosen_for_all(),
            Hit::Inspector => return false,
        }
        true
    }

    pub(crate) fn go(&mut self, location: Location) {
        self.commit_rename();
        self.search = None;
        self.browser.go(location);
        self.selection_changed();
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        let handled = self.route(event);
        self.after_ui();
        handled
    }

    pub(crate) fn files_pointer_down(&mut self, pointer: &Pointer) -> bool {
        self.pointer = pointer.position;
        self.click(pointer)
    }

    pub(crate) fn files_hover(&mut self, pointer: &Pointer) -> bool {
        self.pointer = pointer.position;
        let hover = self.hit(pointer.position).filter(|hit| {
            matches!(
                hit,
                Hit::Place(_) | Hit::Eject(_) | Hit::Segment(_) | Hit::CancelJob(_)
            )
        });
        let changed = hover != self.hover;
        self.hover = hover;
        changed
    }

    pub(crate) fn files_scroll(&mut self, scroll: &Scroll) -> bool {
        self.pointer = scroll.position;
        self.scroll(scroll)
    }

    pub(crate) fn leave(&mut self) -> bool {
        self.hover.take().is_some()
    }

    pub(crate) fn choose(&mut self, action: &str) {
        let Some(sheet) = self.sheet.take() else {
            return;
        };
        match sheet {
            Sheet::Conflict { apply_to_all } => {
                let resolution = match action {
                    "replace" => Resolution::Replace,
                    "skip" => Resolution::Skip,
                    "keep-both" => Resolution::KeepBoth,
                    _ => Resolution::Stop,
                };
                if let Some((index, _)) = self.operations.conflict() {
                    self.operations.answer(
                        index,
                        Answer {
                            resolution,
                            apply_to_all,
                        },
                    );
                }
                if self.operations.conflict().is_some() {
                    self.present_conflict();
                }
            }
            Sheet::GoTo(typed) if action == "go" => self.go_to(&typed),
            Sheet::EmptyTrash if action == "confirm" => {
                self.start(Task::EmptyTrash, Purpose::Erase);
            }
            Sheet::DeleteImmediately { paths, items } if action == "confirm" => {
                if !paths.is_empty() {
                    self.start(Task::Delete { sources: paths }, Purpose::Erase);
                }
                if !items.is_empty() {
                    self.start(Task::Erase { items }, Purpose::Erase);
                }
            }
            _ => {}
        }
    }

    fn go_to(&mut self, typed: &str) {
        let typed = typed.trim();
        let expanded = match typed.strip_prefix('~') {
            Some(rest) => home().join(rest.trim_start_matches('/')),
            None => PathBuf::from(typed),
        };
        let path = if expanded.is_absolute() {
            expanded
        } else {
            self.browser
                .current_folder()
                .unwrap_or_else(home)
                .join(expanded)
        };
        if path.is_dir() {
            self.go(Location::Folder(path));
        } else if path.symlink_metadata().is_ok() {
            self.commit_rename();
            self.browser.reveal(&path);
            self.selection_changed();
        } else {
            self.show_notice(
                L10N.tr_args("error-go-to", [("path", path.display().to_string().into())]),
            );
        }
    }

    pub(crate) fn rename_key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        match key {
            Key::Enter => self.commit_rename(),
            Key::Escape => self.cancel_rename(),
            Key::Tab => {
                self.commit_rename();
                if modifiers.shift {
                    self.browser.up();
                } else {
                    self.browser.down();
                }
                self.selection_changed();
                self.start_rename();
            }
            _ => return self.ui.handle(&Event::Key { key, modifiers }),
        }
        true
    }

    fn type_select(&mut self, character: char) -> bool {
        let now = Instant::now();
        let fresh = self
            .typed
            .1
            .is_none_or(|last| now.duration_since(last) > TYPE_SELECT_PAUSE);
        if fresh {
            self.typed.0.clear();
        }
        self.typed.0.extend(character.to_lowercase());
        self.typed.1 = Some(now);
        let typed = self.typed.0.clone();
        let column = self.browser.active_column();
        let found = column
            .entries
            .iter()
            .position(|entry| entry.name.to_lowercase().starts_with(&typed))
            .or_else(|| {
                column
                    .entries
                    .iter()
                    .position(|entry| entry.name.to_lowercase() >= typed)
            });
        if let Some(row) = found {
            self.browser.select(self.browser.active(), row);
            self.selection_changed();
        }
        true
    }

    pub(crate) fn key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        let command = modifiers.logo;
        let shift = modifiers.shift;
        let alt = modifiers.alt;
        match key {
            Key::Space if !command => self.toggle_quick_look(),
            Key::Escape if self.quick_look.is_some() => self.quick_look = None,
            Key::Escape if self.search.is_some() => self.close_search(),
            Key::Character('f' | 'F') if command => self.open_search(),
            Key::Character('.' | '>') if command && shift => self.toggle_hidden(),
            Key::Function(10) if shift => self.keyboard_menu(),
            Key::Up if command => {
                self.browser.enclosing_folder();
                self.selection_changed();
            }
            Key::Down if command => self.open(),
            Key::Character('o' | 'O') if command && !shift => self.open(),
            Key::Enter if !command => self.open(),
            Key::Function(2) => self.start_rename(),
            Key::Down if shift => {
                self.browser.extend(true);
                self.selection_changed();
            }
            Key::Up if shift => {
                self.browser.extend(false);
                self.selection_changed();
            }
            Key::Down => {
                self.browser.down();
                self.selection_changed();
            }
            Key::Up => {
                self.browser.up();
                self.selection_changed();
            }
            Key::Right => {
                if !self.browser.right() {
                    return false;
                }
                self.selection_changed();
            }
            Key::Left => {
                if !self.browser.left() {
                    return false;
                }
                self.selection_changed();
            }
            Key::Character('[') if command => {
                self.browser.go_back();
                self.selection_changed();
            }
            Key::Character(']') if command => {
                self.browser.go_forward();
                self.selection_changed();
            }
            Key::Backspace if command && shift => self.ask_to_empty_trash(),
            Key::Backspace if command && alt => self.confirm_delete(),
            Key::Backspace if command => self.trash_or_put_back(),
            Key::Character('z' | 'Z') if command && shift => self.redo(),
            Key::Character('z' | 'Z') if command => self.undo(),
            Key::Character('n' | 'N') if command && shift => self.new_folder(),
            Key::Character('g' | 'G') if command && shift => self.ask_go_to(),
            Key::Character('d' | 'D') if command && shift => {
                self.go(Location::Folder(user_folder(UserDir::Desktop)));
            }
            Key::Character('d' | 'D') if command => self.duplicate(),
            Key::Character('e' | 'E') if command => self.eject_current(),
            Key::Character('h' | 'H') if command && shift => {
                self.go(Location::Folder(home()));
            }
            Key::Character('o' | 'O') if command && shift => {
                self.go(Location::Folder(user_folder(UserDir::Documents)));
            }
            Key::Character('l' | 'L') if command && alt => {
                self.go(Location::Folder(user_folder(UserDir::Downloads)));
            }
            Key::Character('2') if command => self.set_list(true),
            Key::Character('3') if command => self.set_list(false),
            Key::Character('i' | 'I') if command => {
                self.inspector = !self.inspector;
                self.selection_changed();
            }
            Key::Character('a' | 'A') if command => {
                self.browser.select_all();
                self.selection_changed();
            }
            Key::Character('c' | 'C') if command => self.copy_selection(false),
            Key::Character('x' | 'X') if command => self.copy_selection(true),
            Key::Character('v' | 'V') if command => self.paste(alt),
            Key::Character(character) if !command && !modifiers.control => {
                return self.type_select(character);
            }
            _ => return false,
        }
        true
    }

    pub fn show_inspector(&mut self) {
        self.inspector = true;
        self.selection_changed();
    }

    pub(crate) fn toggle_hidden(&mut self) {
        self.browser.toggle_hidden();
        self.selection_changed();
    }

    pub(crate) fn ask_to_empty_trash(&mut self) {
        if !self.browser.trash().is_empty() {
            self.present(Sheet::EmptyTrash);
        }
    }

    pub(crate) fn set_list(&mut self, list: bool) {
        self.commit_rename();
        self.browser.set_single(list);
        self.selection_changed();
        self.focus_content();
    }

    pub(crate) fn paste(&mut self, moves: bool) {
        self.paste_moves = Some(moves);
        self.requests.push(Request::PasteFiles);
    }

    pub fn search(&self) -> Option<&Search> {
        self.search.as_ref()
    }

    pub(crate) fn apply_search(&mut self, text: &str) {
        if let Some(search) = &self.search {
            let column = search.column;
            self.browser.set_filter(column, text);
            self.selection_changed();
        }
    }

    pub(crate) fn search_key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        match key {
            Key::Escape => self.close_search(),
            Key::Enter | Key::Tab => {
                self.focus_content();
                if key == Key::Enter && self.browser.selected().is_some() {
                    self.open();
                }
            }
            Key::Up | Key::Down => {
                let column = self.search.as_ref().map_or(0, |search| search.column);
                if self.browser.active() == column {
                    if key == Key::Up {
                        self.browser.up();
                    } else {
                        self.browser.down();
                    }
                    self.selection_changed();
                }
            }
            _ => return self.ui.handle(&Event::Key { key, modifiers }),
        }
        true
    }

    pub(crate) fn toggle_quick_look(&mut self) {
        self.quick_look = match self.quick_look.take() {
            Some(_) => None,
            None => self.browser.selected().cloned().map(|entry| QuickLook {
                entry,
                preview: None,
            }),
        };
        if let Some(quick_look) = &self.quick_look {
            self.loader.preview(quick_look.entry.clone());
        }
    }

    pub(crate) fn start(&mut self, task: Task, purpose: Purpose) {
        if let Err(error) = self.operations.start(task, purpose) {
            self.show_notice(
                L10N.tr_args("error-operation", [("error", error.to_string().into())]),
            );
        }
    }

    pub(crate) fn open(&mut self) {
        let selection = self
            .browser
            .selection()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        if let [entry] = selection.as_slice()
            && entry.is_folder()
        {
            self.go(Location::Folder(entry.path.clone()));
            return;
        }
        let mut launches: Vec<(DesktopEntry, Vec<PathBuf>)> = Vec::new();
        for entry in selection.iter().filter(|entry| !entry.is_folder()) {
            let mime = self.apps.mime_for(&entry.path);
            if let Some(launcher) = self.apps.launcher(&entry.path, &mime) {
                launches.push((launcher, Vec::new()));
                continue;
            }
            match self.apps.default_for(&mime).cloned() {
                Some(app) => match launches.iter_mut().find(|(known, _)| known.id == app.id) {
                    Some((_, files)) => files.push(entry.path.clone()),
                    None => launches.push((app, vec![entry.path.clone()])),
                },
                None => {
                    let kind = self.apps.kind(&mime);
                    self.show_notice(L10N.tr_args(
                        "error-no-app",
                        [("name", entry.name.clone().into()), ("kind", kind.into())],
                    ));
                }
            }
        }
        self.requests
            .extend(launches.into_iter().map(|(app, files)| Request::Open {
                app: Box::new(app),
                files,
            }));
    }

    fn open_with_chosen(&mut self) {
        let Some(info) = &self.info else {
            return;
        };
        let Some(app) = info.chosen.and_then(|index| info.apps.get(index)) else {
            return;
        };
        self.requests.push(Request::Open {
            app: Box::new(app.clone()),
            files: info
                .entries
                .iter()
                .map(|entry| entry.path.clone())
                .collect(),
        });
    }

    fn use_chosen_for_all(&mut self) {
        let Some(info) = &self.info else {
            return;
        };
        let Some(app) = info.chosen.and_then(|index| info.apps.get(index)).cloned() else {
            return;
        };
        let mime = info.mime.clone();
        match self.apps.set_default(&mime, &app) {
            Ok(()) => {
                if let Some(info) = &mut self.info {
                    info.default = Some(app.id.clone());
                }
            }
            Err(error) => self.show_notice(
                L10N.tr_args("error-default-app", [("error", error.to_string().into())]),
            ),
        }
    }

    pub(crate) fn rename_to(&mut self, path: &Path, name: &str) {
        match unbound_fs::rename(path, name) {
            Ok(Some(change)) => {
                if let Change::Moved { to, .. } = &change {
                    self.reveal_after_change(std::slice::from_ref(to));
                }
                self.operations.record(Action::Rename, change);
            }
            Ok(None) => {}
            Err(error) => {
                let message = match error {
                    RenameError::Empty => L10N.tr("rename-empty"),
                    RenameError::ContainsSlash => L10N.tr("rename-slash"),
                    RenameError::Reserved => L10N.tr("rename-reserved"),
                    RenameError::Exists => {
                        L10N.tr_args("rename-exists", [("name", name.to_owned().into())])
                    }
                    RenameError::Io(error) => {
                        L10N.tr_args("error-operation", [("error", error.to_string().into())])
                    }
                };
                self.show_notice(message);
            }
        }
    }

    fn reveal_after_change(&mut self, paths: &[PathBuf]) {
        let folders = paths
            .iter()
            .filter_map(|path| path.parent().map(Path::to_path_buf))
            .collect::<Vec<_>>();
        self.browser.refresh(&folders);
        self.browser.reveal_all(paths);
        self.selection_changed();
    }

    pub(crate) fn new_folder(&mut self) {
        let Some(folder) = self.browser.active_folder().map(Path::to_path_buf) else {
            return;
        };
        let folder = self.browser.current_folder().unwrap_or(folder);
        match unbound_fs::new_folder(&folder) {
            Ok(change) => {
                if let Change::Created(path) = &change {
                    self.reveal_after_change(std::slice::from_ref(path));
                }
                self.operations.record(Action::NewFolder, change);
                self.start_rename();
            }
            Err(error) => self.show_notice(
                L10N.tr_args("error-operation", [("error", error.to_string().into())]),
            ),
        }
    }

    pub(crate) fn duplicate(&mut self) {
        let sources = self.browser.selected_paths();
        if !sources.is_empty() && !self.browser.in_trash() {
            self.start(Task::Duplicate { sources }, Purpose::Do(Action::Duplicate));
        }
    }

    pub(crate) fn trash_or_put_back(&mut self) {
        let selection = self.browser.selection();
        let items = selection
            .iter()
            .filter_map(|entry| entry.trashed.clone())
            .collect::<Vec<_>>();
        if !items.is_empty() {
            self.start(Task::Restore { items }, Purpose::Do(Action::PutBack));
            return;
        }
        let sources = self.browser.selected_paths();
        if sources.is_empty() || self.browser.trash().contains(&sources[0]) {
            return;
        }
        self.start(Task::Trash { sources }, Purpose::Do(Action::Trash));
    }

    pub(crate) fn confirm_delete(&mut self) {
        let selection = self.browser.selection();
        let items = selection
            .iter()
            .filter_map(|entry| entry.trashed.clone())
            .collect::<Vec<_>>();
        let paths = selection
            .iter()
            .filter(|entry| entry.trashed.is_none())
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        if !items.is_empty() || !paths.is_empty() {
            self.present(Sheet::DeleteImmediately { paths, items });
        }
    }

    fn undo(&mut self) {
        if let Err(error) = self.operations.undo() {
            self.show_notice(
                L10N.tr_args("error-operation", [("error", error.to_string().into())]),
            );
        }
    }

    fn redo(&mut self) {
        if let Err(error) = self.operations.redo() {
            self.show_notice(
                L10N.tr_args("error-operation", [("error", error.to_string().into())]),
            );
        }
    }

    pub(crate) fn copy_selection(&mut self, cut: bool) {
        let paths = self.browser.selected_paths();
        if !paths.is_empty() && !(cut && self.browser.in_trash()) {
            self.requests.push(Request::CopyFiles { paths, cut });
        }
    }

    pub fn set_clipboard_has_files(&mut self, files: bool) {
        self.clipboard_files = files;
    }

    pub fn pasted(&mut self, text: &str) -> bool {
        let handled = self.ui.pasted(text);
        self.after_ui();
        handled
    }

    pub fn pasted_files(&mut self, uris: &[String], text: &str, cut: bool) -> bool {
        let moves = self.paste_moves.take().unwrap_or(false) || cut;
        let mut sources = uris
            .iter()
            .filter_map(|uri| path_from_file_uri(uri))
            .collect::<Vec<_>>();
        if sources.is_empty() {
            sources = pasted_paths(text);
        }
        let Some(destination) = self
            .browser
            .current_folder()
            .filter(|_| !self.browser.in_trash())
        else {
            return false;
        };
        if sources.is_empty() {
            return false;
        }
        let task = if moves {
            Task::Move {
                sources,
                destination,
            }
        } else {
            Task::Copy {
                sources,
                destination,
            }
        };
        let action = if moves { Action::Move } else { Action::Copy };
        self.start(task, Purpose::Do(action));
        true
    }

    fn finished(&mut self, finished: Finished) {
        let outcome = finished.outcome;
        if let Some(failure) = outcome.failures.first() {
            let name = failure
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.show_notice(L10N.tr_args(
                "error-item",
                [
                    ("name", name.into()),
                    ("error", failure.error.to_string().into()),
                    ("others", (outcome.failures.len() - 1).into()),
                ],
            ));
        }
        if matches!(self.sheet, Some(Sheet::Conflict { .. }))
            && self.operations.conflict().is_none()
        {
            self.dismiss_sheet();
        }
        let shown = outcome
            .changes
            .iter()
            .filter_map(|change| match change {
                Change::Created(path) | Change::Moved { to: path, .. } => Some(path.clone()),
                Change::Trashed(_) => None,
            })
            .collect::<Vec<_>>();
        let touched = outcome
            .changes
            .iter()
            .flat_map(|change| match change {
                Change::Created(path) => vec![path.clone()],
                Change::Moved { from, to } => vec![from.clone(), to.clone()],
                Change::Trashed(item) => vec![item.original.clone()],
            })
            .filter_map(|path| path.parent().map(Path::to_path_buf))
            .chain([self.browser.trash().home().root().join("info")])
            .collect::<Vec<_>>();
        self.browser.refresh(&touched);
        self.trash_full = !self.browser.trash().is_empty();
        let reveal = matches!(
            finished.purpose,
            Purpose::Do(Action::Copy | Action::Move | Action::Duplicate | Action::PutBack)
                | Purpose::Undo(_)
                | Purpose::Redo(_)
        );
        let in_view = shown
            .first()
            .and_then(|path| path.parent())
            .is_some_and(|parent| {
                self.browser
                    .columns()
                    .iter()
                    .any(|column| column.folder() == Some(parent))
            });
        if reveal && in_view && !shown.is_empty() {
            self.browser.reveal_all(&shown);
        }
        self.selection_changed();
    }
}
