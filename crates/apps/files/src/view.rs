use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::os::fd::BorrowedFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use bande_fs::{
    Action, Answer, Change, Entry, Finished, FolderChanges, FolderWatch, Operations, Purpose,
    RenameError, Resolution, Sort, Step, Task, TrashItem, archive_path, can_extract,
};
use bande_preview::Preview;
use bande_search::{Usage, recent_files};
use bande_thumbnails::Thumbnails;
use bande_toolkit::ui::{
    ClipboardRequest, Context, Cursor, Event, FreeLayout, Key, Menu, Modifiers, Pointer,
    PointerButton, Scroll, ScrollState, Slider, Ui, Widget, WidgetId,
};
use bande_toolkit::{Appearance, Image, Item, Palette, Point, Rect, SHEET_INSET, TextSystem};
use bande_volumes::mounted_volumes;
use bande_xdg::{Apps, DesktopEntry, UserDir, path_from_file_uri, user_dir};

use crate::L10N;
use crate::browser::{Browser, Location};
use crate::controls::{Formats, Press, Signals};
use crate::grid::Band;
use crate::icons::Icons;
use crate::loader::{Loaded, Loader};
use crate::quick_look::QuickLook;
use crate::search::{Index, SEARCH_HEIGHT, Searching};
use crate::settings::FilesSettings;
use crate::sidebar::{Place, PlaceKind, Row, SidebarEdit, with_volumes};
use crate::tabs::{Tab, location_title};
use crate::thumbs::Thumbs;
use crate::toolbar::{TOOLBAR_HEIGHT, Toolbar};
use crate::views::{
    DEFAULT_ICON_SIZE, FolderView, ICON_SIZES, ListColumn, ViewMemory, ViewMode, load_list_columns,
};

pub const COLUMN_WIDTH: f32 = 240.0;
pub const ROW_HEIGHT: f32 = 32.0;
pub const PREVIEW_WIDTH: f32 = 280.0;
pub const SIDEBAR_WIDTH: f32 = 200.0;
pub const INSPECTOR_WIDTH: f32 = 260.0;
pub const PATH_BAR_HEIGHT: f32 = 28.0;
pub const PROGRESS_HEIGHT: f32 = 48.0;
pub const TOP_INSET: f32 = 6.0;
pub const HEADER_HEIGHT: f32 = 26.0;
pub(crate) const SLOW_CLICK_RENAME: Duration = Duration::from_millis(600);
const TYPE_SELECT_PAUSE: Duration = Duration::from_millis(1000);
const NOTICE_TIME: Duration = Duration::from_secs(5);
const MAX_RECENTS: usize = 200;
const HIDDEN: &str = "hidden";
const TRASH_SOUND: &str = "trash-empty";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Watch,
    Listing,
    Loader,
    Thumbnails,
    Index,
    Job(usize),
}

pub struct ColumnPreview {
    pub entry: Option<Entry>,
    pub count: usize,
    pub bytes: u64,
    pub kind: String,
    pub created: Option<SystemTime>,
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
    pub pixels: Option<(u32, u32)>,
    pub mode: Option<u32>,
    pub owned: bool,
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
    OpenWith(crate::open_with::OpenWith),
    Rename(crate::batch_rename::BatchRename),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    Place(usize),
    Eject(usize),
    Row { column: usize, row: usize },
    Column(usize),
    Inspector,
    Sidebar,
    Segment(PathBuf),
    CancelJob(usize),
    ChooseApp(usize),
    OpenWithChosen,
    UseForAll,
    MoreInfo,
    Tab(usize),
    CloseTab(usize),
    NewTab,
    QuickLook,
    QuickLookOpen,
    QuickLookFullScreen,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Open {
        app: Box<DesktopEntry>,
        files: Vec<PathBuf>,
    },
    Clipboard(ClipboardRequest),
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
    EditSidebar(SidebarEdit),
    Eject(PathBuf),
    NewWindow,
    OpenTerminal(PathBuf),
    ShowEmojiPicker,
    PlaySound(&'static str),
    ShowHidden(bool),
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
    pub(crate) table_synced: Option<(u64, usize)>,
    pub(crate) table_formats: Formats,
    pub(crate) table_revealed: Option<usize>,
    pub(crate) table_shape: (bool, Vec<ListColumn>),
    pub(crate) list_columns: Vec<ListColumn>,
    pub(crate) list_columns_file: Option<PathBuf>,
    pub(crate) press: Option<Press>,
    pub(crate) drag_sources: Vec<PathBuf>,
    pub(crate) reordering: Option<Location>,
    pub(crate) status_bar: bool,
    status_bar_file: Option<PathBuf>,
    templates: Option<PathBuf>,
    pub(crate) drop: Option<crate::drag::Hover>,
    pub(crate) scrolls: Vec<(Location, ScrollState)>,
    pub(crate) tabs: Vec<Option<Tab>>,
    pub(crate) active_tab: usize,
    pub(crate) sheet_cancel: Option<WidgetId>,
    pub(crate) size: (f32, f32),
    pub(crate) quick_look: Option<QuickLook>,
    pub(crate) inspector: bool,
    pub(crate) info: Option<Info>,
    pub(crate) inspector_controls: Option<crate::inspector::InspectorControls>,
    pub(crate) inspector_scroll: ScrollState,
    pub(crate) column_preview: Option<ColumnPreview>,
    pub(crate) rename: Option<Rename>,
    pub(crate) sheet: Option<Sheet>,
    pub(crate) notice: Option<(String, Instant)>,
    pub(crate) hits: Vec<(Rect, Hit)>,
    pub(crate) operations: Operations,
    pub(crate) trash_full: bool,
    pub(crate) hover: Option<Hit>,
    pub(crate) searching: Option<Searching>,
    pub(crate) index: Option<Index>,
    pub(crate) toolbar: Toolbar,
    pub(crate) sheet_inset: f32,
    recents: Option<(PathBuf, PathBuf)>,
    pub(crate) search_excluded: Vec<PathBuf>,
    pub(crate) menu_place: Option<usize>,
    pub(crate) hidden_places: Vec<Location>,
    media: Vec<PathBuf>,
    pub(crate) favourites: Vec<Row>,
    watch: FolderWatch,
    pub(crate) loader: Loader,
    pub(crate) thumbs: Thumbs,
    pub(crate) mode: ViewMode,
    pub(crate) icon_size: u32,
    pub(crate) show_hidden: bool,
    pub(crate) folders_first: bool,
    warn_empty_trash: bool,
    pub(crate) search_home: bool,
    views: ViewMemory,
    pub(crate) shown_root: Option<Location>,
    pub(crate) grid_scroll: (Option<Location>, ScrollState),
    pub(crate) band: Option<Band>,
    pub(crate) labels: HashMap<(String, u32, usize), String>,
    pub(crate) grid_wanted: Vec<Entry>,
    pub(crate) size_slider: Option<WidgetId>,
    pub(crate) requests: Vec<Request>,
    pub(crate) pending_rename: Option<(Instant, PathBuf)>,
    pointer: Point,
    typed: (String, Option<Instant>),
    pub(crate) paste_moves: Option<bool>,
    pub(crate) clipboard_files: bool,
}

fn step_title(step: &Step) -> String {
    let id = match step.action {
        Action::Copy => "edit-copy",
        Action::Move => "edit-move",
        Action::Duplicate => "edit-duplicate",
        Action::Rename => "edit-rename",
        Action::NewFolder => "edit-new-folder",
        Action::NewDocument => "edit-new-document",
        Action::Trash => "edit-trash",
        Action::PutBack => "edit-put-back",
        Action::Compress => "edit-compress",
        Action::Extract => "edit-extract",
    };
    L10N.tr_args(id, [("count", step.changes.len().into())])
}

pub(crate) fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

fn user_folder(dir: UserDir) -> PathBuf {
    let home = home();
    let user_dirs = bande_xdg::config_home()
        .and_then(|config| std::fs::read_to_string(config.join("user-dirs.dirs")).ok());
    user_dir(&home, user_dirs.as_deref(), dir)
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
            table_synced: None,
            table_formats: Formats::default(),
            table_revealed: None,
            table_shape: (false, ListColumn::DEFAULT.to_vec()),
            list_columns: ListColumn::DEFAULT.to_vec(),
            list_columns_file: None,
            press: None,
            drag_sources: Vec::new(),
            reordering: None,
            status_bar: true,
            status_bar_file: None,
            templates: None,
            drop: None,
            scrolls: Vec::new(),
            tabs: vec![None],
            active_tab: 0,
            sheet_cancel: None,
            size: (0.0, 0.0),
            quick_look: None,
            inspector: false,
            info: None,
            inspector_controls: None,
            inspector_scroll: ScrollState::default(),
            column_preview: None,
            rename: None,
            sheet: None,
            notice: None,
            hits: Vec::new(),
            operations,
            trash_full: false,
            hover: None,
            searching: None,
            index: None,
            toolbar: Toolbar::default(),
            sheet_inset: SHEET_INSET as f32,
            recents: None,
            search_excluded: Vec::new(),
            menu_place: None,
            hidden_places: Vec::new(),
            media: Vec::new(),
            favourites: Vec::new(),
            watch: FolderWatch::new()?,
            loader: Loader::new()?,
            thumbs: Thumbs::default(),
            mode: ViewMode::default(),
            icon_size: DEFAULT_ICON_SIZE,
            show_hidden: false,
            folders_first: true,
            warn_empty_trash: true,
            search_home: false,
            views: ViewMemory::default(),
            shown_root: None,
            grid_scroll: (None, ScrollState::default()),
            band: None,
            labels: HashMap::new(),
            grid_wanted: Vec::new(),
            size_slider: None,
            requests: Vec::new(),
            pending_rename: None,
            pointer: Point::default(),
            typed: (String::new(), None),
            paste_moves: None,
            clipboard_files: false,
        };
        view.shown_root = Some(view.browser.root().clone());
        view.insert_toolbar();
        view.ui.set_sheet_top(TOOLBAR_HEIGHT);
        view.favourites = view.sidebar.clone();
        view.watch.watch(&view.watched());
        view.trash_full = !view.browser.trash().is_empty();
        view.focus_content();
        Ok(view)
    }

    pub fn set_hidden_places(&mut self, hidden: Vec<Location>) {
        self.hidden_places = hidden;
        let favourites = std::mem::take(&mut self.favourites);
        self.set_sidebar(favourites);
    }

    pub fn set_sidebar(&mut self, sidebar: Vec<Row>) {
        self.sidebar = self.with_volumes(&sidebar, &self.volumes());
        self.favourites = sidebar;
        self.menu_place = None;
    }

    pub fn set_media(&mut self, roots: Vec<PathBuf>) {
        self.media = roots;
        self.refresh_volumes();
        self.watch.watch(&self.watched());
    }

    fn with_volumes(&self, sidebar: &[Row], volumes: &[PathBuf]) -> Vec<Row> {
        let shown = volumes
            .iter()
            .filter(|volume| {
                !self
                    .hidden_places
                    .contains(&Location::Folder((*volume).clone()))
            })
            .cloned()
            .collect::<Vec<_>>();
        with_volumes(sidebar, &shown)
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
        self.sidebar = self.with_volumes(&self.favourites, &volumes);
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

    pub fn set_window_active(&mut self, active: bool) -> bool {
        self.ui.set_window_active(active)
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.palette = appearance.palette();
        self.ui.set_appearance(appearance);
        self.layout_controls();
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

    pub fn column_preview(&self) -> Option<&ColumnPreview> {
        self.column_preview.as_ref()
    }

    fn update_column_preview(&mut self) {
        let column = self.browser.active_column();
        let shown = self.mode == ViewMode::Columns
            && !self.browser.single()
            && self.browser.active() + 1 == self.browser.columns().len()
            && !column.marked.is_empty()
            && self
                .browser
                .selected()
                .is_none_or(|entry| !entry.is_folder());
        if !shown {
            self.column_preview = None;
            return;
        }
        let entry = self.browser.selected().cloned();
        let kept = self
            .column_preview
            .take()
            .filter(|kept| kept.entry.is_some() && kept.entry == entry);
        if let Some(kept) = kept {
            self.column_preview = Some(kept);
            return;
        }
        let bytes = column
            .marked
            .iter()
            .filter_map(|row| column.entries.get(*row))
            .filter(|entry| !entry.is_folder())
            .map(|entry| entry.size)
            .sum();
        let count = column.marked.len();
        let kind = entry.as_ref().map_or_else(String::new, |entry| {
            self.apps.kind_for_name(&entry.name, entry.is_folder())
        });
        let created = entry.as_ref().and_then(|entry| {
            let born = std::fs::metadata(&entry.path)
                .and_then(|metadata| metadata.created())
                .ok();
            created_at(born, entry.modified)
        });
        self.column_preview = Some(ColumnPreview {
            entry,
            count,
            bytes,
            kind,
            created,
            preview: None,
        });
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
        match self.browser.single() {
            true => location_title(&self.browser.active_column().location),
            false => location_title(&self.browser.current_location()),
        }
    }

    pub fn set_thumbnails(&mut self, thumbnails: Thumbnails) {
        self.thumbs.set_service(thumbnails);
    }

    pub fn set_view_memory(&mut self, views: ViewMemory) {
        self.views = views;
        self.shown_root = None;
        self.selection_changed();
    }

    pub fn mode(&self) -> ViewMode {
        self.mode
    }

    pub fn icon_size(&self) -> u32 {
        self.icon_size
    }

    pub fn set_recents_sources(&mut self, xbel: PathBuf, usage: PathBuf) {
        self.recents = Some((xbel, usage));
        self.load_recents();
    }

    pub fn set_search_excluded(&mut self, excluded: Vec<PathBuf>) {
        if excluded == self.search_excluded {
            return;
        }
        self.search_excluded = excluded;
        if let Some(index) = &self.index {
            index.index.exclude(self.search_excluded.clone());
        }
        self.load_recents();
        self.query();
    }

    fn load_recents(&mut self) {
        let Some((xbel, usage)) = &self.recents else {
            return;
        };
        let shared = std::fs::read_to_string(xbel).ok();
        let usage = Usage::load(usage.clone());
        self.browser.set_recents(recent_files(
            shared.as_deref(),
            &usage,
            &self.search_excluded,
            MAX_RECENTS,
        ));
    }

    fn apply_remembered_view(&mut self) {
        let root = self.browser.root().clone();
        if self.shown_root.as_ref() == Some(&root) {
            return;
        }
        self.shown_root = Some(root.clone());
        if root == Location::Recents {
            self.load_recents();
        }
        let Location::Folder(folder) = &root else {
            return;
        };
        if let Some(view) = self.views.get(folder) {
            self.icon_size = view.icon_size;
            let sort = Sort {
                folders_first: self.folders_first,
                ..view.sort
            };
            if sort != self.browser.sort() {
                self.browser.set_sort(sort);
            }
            if view.mode != self.mode {
                self.mode = view.mode;
                self.browser.set_single(view.mode != ViewMode::Columns);
                self.shown_root = Some(self.browser.root().clone());
                self.focus_content();
            }
            self.refresh_size_slider();
            self.refresh_view_control();
        }
    }

    pub(crate) fn remember_view(&mut self) {
        let Location::Folder(folder) = self.browser.root().clone() else {
            return;
        };
        let view = FolderView {
            mode: self.mode,
            icon_size: self.icon_size,
            sort: self.browser.sort(),
        };
        if let Err(error) = self.views.remember(&folder, view) {
            tracing::warn!("cannot remember the view of {}: {error}", folder.display());
        }
    }

    fn sources(&self) -> Vec<(Source, BorrowedFd<'_>)> {
        let mut sources = vec![(Source::Watch, self.watch.fd())];
        sources.extend(self.browser.listing_fd().map(|fd| (Source::Listing, fd)));
        sources.push((Source::Loader, self.loader.fd()));
        sources.extend(self.thumbs.fd().map(|fd| (Source::Thumbnails, fd)));
        sources.extend(self.search_index_fd().map(|fd| (Source::Index, fd)));
        sources.extend(
            self.operations
                .fds()
                .into_iter()
                .enumerate()
                .map(|(job, fd)| (Source::Job(job), fd)),
        );
        sources
    }

    pub fn fds(&self) -> Vec<BorrowedFd<'_>> {
        self.sources().into_iter().map(|(_, fd)| fd).collect()
    }

    pub fn source_ready(&mut self, index: usize) -> bool {
        let Some(source) = self.sources().get(index).map(|(source, _)| *source) else {
            return false;
        };
        match source {
            Source::Watch => {
                self.watch.read();
                false
            }
            Source::Listing => self.listed(),
            Source::Loader => self.loaded(),
            Source::Thumbnails => self.thumbs.take(),
            Source::Index => self.search_results(),
            Source::Job(job) => match self.operations.poll(job) {
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
        let changes = self.watch.take_changed(now);
        if !changes.is_empty() {
            self.apply_changes(&changes);
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
        self.apply_changes(&FolderChanges {
            folders: changed.to_vec(),
            entries: Vec::new(),
        });
    }

    fn apply_changes(&mut self, changes: &FolderChanges) {
        let folders = changes
            .folders
            .iter()
            .map(PathBuf::as_path)
            .chain(changes.entries.iter().filter_map(|path| path.parent()));
        if folders
            .into_iter()
            .any(|path| self.media.iter().any(|root| root.starts_with(path)))
        {
            self.refresh_volumes();
        }
        let editing = self.rename.as_ref().map(|rename| rename.path.clone());
        self.trash_full = !self.browser.trash().is_empty();
        self.browser.apply(changes);
        if let Some(path) = editing
            && path.symlink_metadata().is_err()
        {
            self.cancel_rename();
        }
        self.selection_changed();
    }

    pub(crate) fn show_notice(&mut self, text: String) {
        self.notice = Some((text, Instant::now()));
    }

    fn listed(&mut self) -> bool {
        match self.browser.take_listed() {
            (_, true) => self.selection_changed(),
            (true, false) => self.sync_table(),
            (false, false) => return false,
        }
        true
    }

    fn loaded(&mut self) -> bool {
        let mut changed = false;
        for loaded in self.loader.take() {
            match loaded {
                Loaded::Preview {
                    path,
                    preview,
                    pixels,
                } => {
                    if let Some(shown) = &mut self.column_preview
                        && shown.entry.as_ref().is_some_and(|entry| entry.path == path)
                    {
                        shown.preview = Some(preview.clone());
                        changed = true;
                    }
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
                        info.pixels = pixels;
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
        self.apply_remembered_view();
        self.browser.set_nesting(self.mode == ViewMode::List);
        self.watch.watch(&self.watched());
        self.loader.forget_pending();
        let entry = self.browser.selected().cloned();
        self.follow_selection_in_quick_look(entry.as_ref());
        let quick_look = self
            .quick_look
            .as_ref()
            .filter(|shown| shown.preview.is_none())
            .map(|shown| shown.entry.clone());
        if self.inspector {
            self.update_info();
        } else {
            self.info = None;
            self.sync_inspector_controls();
        }
        self.update_column_preview();
        let column_preview = self
            .column_preview
            .as_ref()
            .is_some_and(|shown| shown.entry.is_some() && shown.preview.is_none());
        let quick_look_path = quick_look.as_ref().map(|shown| shown.path.clone());
        if let Some(shown) = quick_look {
            self.loader.preview(shown);
        }
        if let Some(entry) = entry
            && (self.inspector || column_preview)
        {
            if quick_look_path.as_ref() != Some(&entry.path) {
                self.loader.preview(entry.clone());
            }
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
        let metadata = match entries.as_slice() {
            [entry] if entry.trashed.is_none() => std::fs::symlink_metadata(&entry.path).ok(),
            _ => None,
        };
        let mode = metadata.as_ref().map(|metadata| metadata.mode());
        let owned = metadata
            .as_ref()
            .is_some_and(|metadata| metadata.uid() == rustix::process::getuid().as_raw());
        let previous = self.info.take();
        if previous.as_ref().is_none_or(|info| info.entries != entries) {
            self.inspector_scroll = ScrollState::default();
        }
        if let Some(mut info) = previous.filter(|info| info.entries == entries) {
            info.mode = mode;
            info.owned = owned;
            self.info = Some(info);
            self.sync_inspector_controls();
            return;
        }
        let mime = match entries.as_slice() {
            [entry] => self.apps.mime_for(&entry.path),
            _ => String::new(),
        };
        let kinds = match entries.len() > 1 {
            true => {
                let mut kinds: Vec<String> = Vec::new();
                for entry in &entries {
                    let kind = self.apps.kind_for_name(&entry.name, entry.is_folder());
                    if !kinds.contains(&kind) {
                        kinds.push(kind);
                    }
                }
                kinds.join(", ")
            }
            false => String::new(),
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
                kinds
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
            pixels: None,
            mode,
            owned,
        });
        self.sync_inspector_controls();
    }

    pub fn set_full_screen(&mut self, full_screen: bool) {
        self.sheet_inset = match full_screen {
            true => 0.0,
            false => SHEET_INSET as f32,
        };
    }

    pub fn sheet_frame(&self) -> Rect {
        Rect::new(
            SIDEBAR_WIDTH,
            TOOLBAR_HEIGHT,
            (self.size.0 - SIDEBAR_WIDTH - self.sheet_inset).max(0.0),
            (self.size.1 - TOOLBAR_HEIGHT - self.sheet_inset).max(0.0),
        )
    }

    pub fn content_frame(&self) -> Rect {
        let sheet = self.sheet_frame();
        let inspector = if self.inspector { INSPECTOR_WIDTH } else { 0.0 };
        let progress = self.visible_jobs() as f32 * PROGRESS_HEIGHT;
        let search = if self.searching.is_some() {
            SEARCH_HEIGHT
        } else {
            0.0
        };
        let top = sheet.y + self.tab_bar_height() + search;
        Rect::new(
            sheet.x,
            top,
            (sheet.width - inspector).max(COLUMN_WIDTH),
            (sheet.y + sheet.height - self.status_bar_height() - progress - top).max(ROW_HEIGHT),
        )
    }

    pub(crate) fn status_bar_height(&self) -> f32 {
        match self.status_bar {
            true => PATH_BAR_HEIGHT,
            false => 0.0,
        }
    }

    pub fn set_list_columns_file(&mut self, file: PathBuf) {
        self.list_columns = load_list_columns(&file);
        self.list_columns_file = Some(file);
        self.sync_table();
    }

    pub fn list_columns(&self) -> &[ListColumn] {
        &self.list_columns
    }

    pub fn set_status_bar_file(&mut self, file: PathBuf) {
        self.status_bar = std::fs::read_to_string(&file).map_or(true, |text| text.trim() != HIDDEN);
        self.status_bar_file = Some(file);
        self.refresh_size_slider();
    }

    fn remember_status_bar(&self) -> io::Result<()> {
        let Some(file) = &self.status_bar_file else {
            return Ok(());
        };
        if self.status_bar {
            return match std::fs::remove_file(file) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            };
        }
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(file, format!("{HIDDEN}\n"))
    }

    pub(crate) fn toggle_status_bar(&mut self) {
        self.status_bar = !self.status_bar;
        if let Err(error) = self.remember_status_bar() {
            tracing::warn!("cannot remember the status bar: {error}");
        }
        self.refresh_size_slider();
        self.selection_changed();
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
        let preview = match self.column_preview {
            Some(_) => PREVIEW_WIDTH,
            None => 0.0,
        };
        let fit = (((self.content_frame().width - preview) / COLUMN_WIDTH).floor() as usize).max(1);
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
        if self.mode == ViewMode::Icons {
            self.reveal_in_grid();
            return;
        }
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
        let sheet = self.sheet_frame();
        if self.inspector && scroll.position.x >= sheet.x + sheet.width - INSPECTOR_WIDTH {
            let cx = self.ui.context();
            return self.inspector_scroll.scroll(cx, scroll);
        }
        let content = self.content_frame();
        if !content.contains(scroll.position) {
            return false;
        }
        match self.mode {
            ViewMode::Icons => return self.grid_scrolled(scroll),
            ViewMode::List => return false,
            ViewMode::Columns => {}
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
            .find(|(rect, _)| rect.contains(at))
            .map(|(_, hit)| hit.clone())
    }

    fn click(&mut self, pointer: &Pointer) -> bool {
        match pointer.button {
            PointerButton::Back => {
                self.back();
                return true;
            }
            PointerButton::Forward => {
                self.end_search();
                self.browser.go_forward();
                self.selection_changed();
                return true;
            }
            _ => {}
        }
        let Some(hit) = self.hit(pointer.position) else {
            return false;
        };
        if pointer.button == PointerButton::Secondary {
            return self.context_click(hit, pointer.position);
        }
        if pointer.button == PointerButton::Middle {
            return self.middle_click(hit);
        }
        if !pointer.is_primary() {
            return false;
        }
        self.pending_rename = None;
        match hit {
            Hit::Row { column, row } => {
                let modifiers = pointer.modifiers;
                let plain = !modifiers.control && !modifiers.shift;
                let Some(shown) = self.browser.columns().get(column) else {
                    return false;
                };
                let Some(entry) = shown.entries.get(row) else {
                    return false;
                };
                let path = entry.path.clone();
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
                let folder_in_tab = pointer.count >= 2
                    && modifiers.control
                    && entry.is_folder()
                    && entry.trashed.is_none();
                self.commit_rename();
                if plain && pointer.count == 1 {
                    self.press = Some(Press::Item {
                        at: pointer.position,
                        collapse: keep.then(|| path.clone()),
                    });
                }
                if folder_in_tab {
                    self.open_tab(Location::Folder(path));
                    return true;
                }
                if keep {
                } else if modifiers.control {
                    self.browser.toggle(column, row);
                } else if modifiers.shift {
                    self.browser.extend_to(column, row);
                } else {
                    self.browser.select(column, row);
                }
                self.selection_changed();
                if pointer.count >= 2 && !modifiers.control && !modifiers.shift {
                    self.open();
                } else if already && pointer.count == 1 && !modifiers.control && !modifiers.shift {
                    self.pending_rename = Some((Instant::now() + SLOW_CLICK_RENAME, path));
                }
            }
            Hit::Column(column) => {
                self.commit_rename();
                if self.mode == ViewMode::Icons {
                    self.start_band(pointer);
                } else {
                    self.browser.focus_column(column);
                }
                self.selection_changed();
            }
            Hit::Place(index) => {
                if let Some(Row::Place(place)) = self.sidebar.get(index) {
                    let location = place.location.clone();
                    self.go(location);
                    self.press = Some(Press::Place {
                        index,
                        at: pointer.position,
                    });
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
            Hit::MoreInfo => self.show_inspector(),
            Hit::Inspector | Hit::Sidebar => return false,
            Hit::Tab(index) => self.select_tab(index),
            Hit::CloseTab(index) => {
                self.close_tab(index);
            }
            Hit::NewTab => self.open_tab(self.browser.root().clone()),
            Hit::QuickLookOpen => self.open_quick_look_item(),
            Hit::QuickLookFullScreen => self.toggle_quick_look_full_screen(),
            Hit::QuickLook => {}
        }
        true
    }

    pub(crate) fn forward(&mut self) {
        self.end_search();
        self.browser.go_forward();
        self.selection_changed();
    }

    pub(crate) fn back(&mut self) {
        if self.searching.is_some() {
            self.end_search();
        } else {
            self.commit_rename();
            self.browser.go_back();
            self.selection_changed();
        }
    }

    fn middle_click(&mut self, hit: Hit) -> bool {
        let location = match hit {
            Hit::Tab(index) | Hit::CloseTab(index) => return self.close_tab(index),
            Hit::Row { column, row } => match self
                .browser
                .columns()
                .get(column)
                .and_then(|shown| shown.entries.get(row))
            {
                Some(entry) if entry.is_folder() && entry.trashed.is_none() => {
                    Location::Folder(entry.path.clone())
                }
                _ => return false,
            },
            Hit::Place(index) => match self.sidebar.get(index) {
                Some(Row::Place(place)) => place.location.clone(),
                _ => return false,
            },
            Hit::Segment(path) if path.is_dir() => Location::Folder(path),
            _ => return false,
        };
        self.open_tab(location);
        true
    }

    pub(crate) fn go(&mut self, location: Location) {
        self.commit_rename();
        self.end_search();
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
                Hit::Place(_)
                    | Hit::Eject(_)
                    | Hit::Segment(_)
                    | Hit::CancelJob(_)
                    | Hit::Tab(_)
                    | Hit::CloseTab(_)
                    | Hit::NewTab
                    | Hit::QuickLookOpen
                    | Hit::QuickLookFullScreen
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
            Sheet::OpenWith(open_with) if action == "open" => self.open_with_chosen_app(open_with),
            Sheet::Rename(state) if action == "rename" => self.apply_batch_rename(state),
            Sheet::EmptyTrash if action == "confirm" => self.empty_trash(),
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
        let folder = self.browser.current_folder().unwrap_or_else(home);
        let path = bande_fs::typed_location(typed, &home(), &folder);
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
        if modifiers.logo {
            return false;
        }
        let command = modifiers.control;
        let shift = modifiers.shift;
        let alt = modifiers.alt;
        let plain = !command && !alt;
        match key {
            Key::Space if plain => self.toggle_quick_look(),
            Key::Function(11) if plain => self.toggle_quick_look_full_screen(),
            Key::Escape if self.searching.is_some() => self.end_search(),
            Key::Character('f' | 'F') if command && shift => self.go(Location::Recents),
            Key::Character('f' | 'F') if command => self.focus_search(),
            Key::Character('h' | 'H') if command && !shift => self.toggle_hidden(),
            Key::Function(10) if shift => self.keyboard_menu(),
            Key::Up if alt => {
                self.browser.enclosing_folder();
                self.selection_changed();
            }
            Key::Down if alt => self.open(),
            Key::Left if alt => self.back(),
            Key::Right if alt => {
                self.end_search();
                self.browser.go_forward();
                self.selection_changed();
            }
            Key::Home if alt => self.go(Location::Folder(home())),
            Key::Character('o' | 'O') if command && !shift => self.open(),
            Key::Enter if plain => self.open(),
            Key::Function(2) => self.start_rename(),
            Key::Up | Key::Down | Key::Left | Key::Right | Key::Home | Key::End
                if self.mode == ViewMode::Icons && !alt =>
            {
                return self.grid_key(key, modifiers);
            }
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
            Key::PageUp if command => self.step_tab(false),
            Key::PageDown if command => self.step_tab(true),
            Key::Tab if command => self.step_tab(!shift),
            Key::Character('d' | 'D') if command && !shift => self.add_to_sidebar(),
            Key::Character('t' | 'T') if command && !shift => {
                self.open_tab(self.browser.root().clone());
            }
            Key::Character('n' | 'N') if command && !shift => {
                self.requests.push(Request::NewWindow)
            }
            Key::Delete if command && shift => self.ask_to_empty_trash(),
            Key::Delete if shift || self.selection_is_trashed() => self.confirm_delete(),
            Key::Delete if plain => self.trash_or_put_back(),
            Key::Character('z' | 'Z') if command && shift => self.redo(),
            Key::Character('z' | 'Z') if command => self.undo(),
            Key::Character('n' | 'N') if command && shift => self.new_folder(),
            Key::Character('l' | 'L') if command && !shift => self.ask_go_to(),
            Key::Character('d' | 'D') if command && shift => {
                self.go(Location::Folder(user_folder(UserDir::Desktop)));
            }
            Key::Character('e' | 'E') if command => self.eject_current(),
            Key::Character('o' | 'O') if command && shift => {
                self.go(Location::Folder(user_folder(UserDir::Documents)));
            }
            Key::Character('l' | 'L') if command && shift => {
                self.go(Location::Folder(user_folder(UserDir::Downloads)));
            }
            Key::Character('/') if command => self.toggle_status_bar(),
            Key::Character('1') if command => self.set_mode(ViewMode::Icons),
            Key::Character('2') if command => self.set_mode(ViewMode::List),
            Key::Character('3') if command => self.set_mode(ViewMode::Columns),
            Key::Character('=' | '+') if command => self.grow_icons(true),
            Key::Character('-') if command => self.grow_icons(false),
            Key::Character('i' | 'I') if command => {
                self.inspector = !self.inspector;
                self.selection_changed();
            }
            Key::Character('a' | 'A') if command => {
                self.browser.select_all();
                self.selection_changed();
            }
            Key::Character('c' | 'C') if command && shift => self.copy_paths(),
            Key::Character('c' | 'C') if command => self.copy_selection(false),
            Key::Character('x' | 'X') if command => self.copy_selection(true),
            Key::Character('v' | 'V') if command => self.paste(shift),
            Key::Character(character) if plain => {
                return self.type_select(character);
            }
            _ => return false,
        }
        true
    }

    fn selection_is_trashed(&self) -> bool {
        self.browser
            .selection()
            .iter()
            .any(|entry| entry.trashed.is_some())
    }

    pub fn show_inspector(&mut self) {
        self.inspector = true;
        self.selection_changed();
    }

    pub(crate) fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.browser.set_show_hidden(self.show_hidden);
        self.requests.push(Request::ShowHidden(self.show_hidden));
        self.selection_changed();
    }

    pub fn apply_settings(&mut self, settings: &FilesSettings) {
        self.warn_empty_trash = settings.warn_empty_trash;
        self.search_home = settings.search_home;
        if settings.folders_first != self.folders_first {
            self.folders_first = settings.folders_first;
            self.browser.set_folders_first(self.folders_first);
            self.selection_changed();
        }
        if settings.show_hidden != self.show_hidden {
            self.show_hidden = settings.show_hidden;
            self.browser.set_show_hidden(self.show_hidden);
            self.selection_changed();
        }
    }

    pub fn set_default_view(&mut self, mode: ViewMode) {
        self.mode = mode;
        self.browser.set_single(mode != ViewMode::Columns);
        self.shown_root = None;
        self.refresh_size_slider();
        self.refresh_view_control();
        self.selection_changed();
    }

    pub(crate) fn ask_to_empty_trash(&mut self) {
        if self.browser.trash().is_empty() {
            return;
        }
        match self.warn_empty_trash {
            true => self.present(Sheet::EmptyTrash),
            false => self.empty_trash(),
        }
    }

    fn empty_trash(&mut self) {
        self.requests.push(Request::PlaySound(TRASH_SOUND));
        self.start(Task::EmptyTrash, Purpose::Erase);
    }

    pub(crate) fn set_mode(&mut self, mode: ViewMode) {
        if self.searching.is_some() {
            self.refresh_view_control();
            return;
        }
        self.commit_rename();
        self.band = None;
        self.mode = mode;
        self.browser.set_single(mode != ViewMode::Columns);
        self.shown_root = Some(self.browser.root().clone());
        self.remember_view();
        self.refresh_size_slider();
        self.refresh_view_control();
        self.selection_changed();
        self.focus_content();
    }

    pub(crate) fn set_icon_size(&mut self, size: u32) {
        let size = size.clamp(ICON_SIZES.0, ICON_SIZES.1);
        if size != self.icon_size {
            self.icon_size = size;
            self.remember_view();
            self.keep_selection_visible();
        }
        let shown = self
            .size_slider
            .and_then(|slider| self.ui.widget_ref::<Slider>(slider))
            .map(|slider| slider.value().round() as u32);
        if shown != Some(self.icon_size) {
            self.refresh_size_slider();
        }
    }

    pub(crate) fn grow_icons(&mut self, bigger: bool) {
        if self.mode != ViewMode::Icons {
            return;
        }
        let step = 16;
        let size = match bigger {
            true => (self.icon_size / step + 1) * step,
            false => (self.icon_size.div_ceil(step) - 1) * step,
        };
        self.set_icon_size(size);
    }

    pub(crate) fn paste(&mut self, moves: bool) {
        self.paste_moves = Some(moves);
        self.requests.push(Request::PasteFiles);
    }

    pub(crate) fn search_key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        match key {
            Key::Escape => {
                self.end_search();
                self.focus_content();
            }
            Key::Enter | Key::Tab => {
                self.focus_content();
                if key == Key::Enter && self.browser.selected().is_some() {
                    self.open();
                }
            }
            Key::Up | Key::Down if self.searching.is_some() => {
                if key == Key::Up {
                    self.browser.up();
                } else {
                    self.browser.down();
                }
                self.selection_changed();
            }
            _ => return self.ui.handle(&Event::Key { key, modifiers }),
        }
        true
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
        let (archives, files): (Vec<_>, Vec<_>) = selection
            .iter()
            .filter(|entry| !entry.is_folder())
            .partition(|entry| entry.trashed.is_none() && can_extract(&entry.path));
        if !archives.is_empty() {
            let archives = archives.iter().map(|entry| entry.path.clone()).collect();
            self.start(Task::Extract { archives }, Purpose::Do(Action::Extract));
        }
        let mut launches: Vec<(DesktopEntry, Vec<PathBuf>)> = Vec::new();
        let mut unknown = Vec::new();
        for entry in files {
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
                None => unknown.push(entry.path.clone()),
            }
        }
        self.requests
            .extend(launches.into_iter().map(|(app, files)| Request::Open {
                app: Box::new(app),
                files,
            }));
        if !unknown.is_empty() {
            self.ask_open_with(unknown);
        }
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
        match bande_fs::rename(path, name) {
            Ok(Some(change)) => {
                if let Change::Moved { to, .. } = &change {
                    self.reveal_after_change(std::slice::from_ref(to));
                }
                self.operations.record(Action::Rename, vec![change]);
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

    pub(crate) fn reveal_after_change(&mut self, paths: &[PathBuf]) {
        let folders = paths
            .iter()
            .filter_map(|path| path.parent().map(Path::to_path_buf))
            .collect::<Vec<_>>();
        self.browser.refresh(&folders);
        self.browser.reveal_all(paths);
        self.selection_changed();
    }

    pub fn set_templates(&mut self, folder: PathBuf) {
        self.templates = Some(folder);
    }

    pub(crate) fn templates(&self) -> Vec<String> {
        let Some(folder) = self.templates.as_ref().filter(|folder| **folder != home()) else {
            return Vec::new();
        };
        let mut names = std::fs::read_dir(folder)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| !name.starts_with('.'))
            .collect::<Vec<_>>();
        names.sort_by(|a, b| bande_fs::finder_order(a, b));
        names
    }

    pub(crate) fn new_document(&mut self, name: &str) {
        let Some(folder) = self.browser.active_folder().map(Path::to_path_buf) else {
            return;
        };
        let Some(template) = self.templates.as_ref().map(|folder| folder.join(name)) else {
            return;
        };
        match bande_fs::new_document(&folder, &template) {
            Ok(change) => {
                if let Change::Created(path) = &change {
                    self.reveal_after_change(std::slice::from_ref(path));
                }
                self.operations.record(Action::NewDocument, vec![change]);
                self.start_rename();
            }
            Err(error) => self.show_notice(
                L10N.tr_args("error-operation", [("error", error.to_string().into())]),
            ),
        }
    }

    pub(crate) fn open_in_terminal(&mut self) {
        let folder = match self.browser.selection().as_slice() {
            [entry] if entry.is_folder() && entry.trashed.is_none() => Some(entry.path.clone()),
            _ => self.browser.active_folder().map(Path::to_path_buf),
        };
        if let Some(folder) = folder {
            self.requests.push(Request::OpenTerminal(folder));
        }
    }

    pub(crate) fn new_folder(&mut self) {
        let Some(folder) = self.browser.active_folder().map(Path::to_path_buf) else {
            return;
        };
        let folder = self.browser.current_folder().unwrap_or(folder);
        match bande_fs::new_folder(&folder) {
            Ok(change) => {
                if let Change::Created(path) = &change {
                    self.reveal_after_change(std::slice::from_ref(path));
                }
                self.operations.record(Action::NewFolder, vec![change]);
                self.start_rename();
            }
            Err(error) => self.show_notice(
                L10N.tr_args("error-operation", [("error", error.to_string().into())]),
            ),
        }
    }

    pub(crate) fn compress(&mut self) {
        let sources = self.browser.selected_paths();
        if self.browser.in_trash() {
            return;
        }
        if let Some(archive) = archive_path(&sources) {
            self.start(
                Task::Compress { sources, archive },
                Purpose::Do(Action::Compress),
            );
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

    pub fn undo_title(&self) -> Option<String> {
        let step = self.operations.history().next_undo()?;
        Some(L10N.tr_args("menu-undo-action", [("action", step_title(step).into())]))
    }

    pub fn redo_title(&self) -> Option<String> {
        let step = self.operations.history().next_redo()?;
        Some(L10N.tr_args("menu-redo-action", [("action", step_title(step).into())]))
    }

    pub(crate) fn copy_paths(&mut self) {
        let paths = self.browser.selected_paths();
        if paths.is_empty() || self.browser.in_trash() {
            return;
        }
        let text = paths
            .iter()
            .map(|path| path.to_string_lossy())
            .collect::<Vec<_>>()
            .join("\n");
        self.requests
            .push(Request::Clipboard(ClipboardRequest::Copy(text)));
    }

    pub(crate) fn add_to_sidebar(&mut self) {
        let selection = self.browser.selection();
        let folders = selection
            .iter()
            .filter(|entry| entry.is_folder() && entry.trashed.is_none())
            .map(|entry| Location::Folder(entry.path.clone()))
            .collect::<Vec<_>>();
        let locations = match (folders.is_empty(), selection.is_empty()) {
            (false, _) => folders,
            (true, true) => self
                .browser
                .current_folder()
                .map(Location::Folder)
                .into_iter()
                .collect(),
            (true, false) => Vec::new(),
        };
        let locations = locations
            .into_iter()
            .filter(|location| {
                !self
                    .favourites
                    .iter()
                    .any(|row| matches!(row, Row::Place(place) if place.location == *location))
            })
            .collect::<Vec<_>>();
        if !locations.is_empty() {
            self.place_in_sidebar(locations, None);
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
            Purpose::Do(
                Action::Copy
                    | Action::Move
                    | Action::Duplicate
                    | Action::PutBack
                    | Action::Compress
                    | Action::Extract
            ) | Purpose::Undo(_)
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

pub(crate) fn created_at(
    born: Option<SystemTime>,
    modified: Option<SystemTime>,
) -> Option<SystemTime> {
    match (born, modified) {
        (Some(born), Some(modified)) => Some(born.min(modified)),
        (born, _) => born,
    }
}
