mod menu_bar;

use std::cell::RefCell;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use bande_app::{Cx, Info, Keys, Kind, Menus, Program, Settings, View, WindowId};
use bande_app_settings::{SettingsWatch, SettingsWindow, Stored, settings_folder};
use bande_files::{
    BookmarkEdit, Browser, FilesSettings, FilesView, L10N, Location, Request, Row, SETTINGS_SIZE,
    SIDEBAR_FILE, SIDEBAR_WIDTH, Show, ShowRequest, SidebarEdit, TOOLBAR_HEIGHT, ViewMemory,
    follow_search_excluded, hidden_places, list_columns_file,
    recent_folders_file as recent_folders_file_in, rows, serve_file_manager, settings_view,
    sidebar_file, status_bar_file, views_file, volumes_hidden, with_bookmarks, with_place_shown,
    without_bookmark,
};
use bande_fs::{History, Trash};
use bande_inbox::Inbox;
use bande_search::{recently_used_path, usage_path};
use bande_thumbnails::Thumbnails;
use bande_toolkit::{
    Appearance, DragAction, DragEvent, FrameRegions, MenuPlacement, Rect, Scene, TextSystem,
    Transfer, Window, WindowEvent,
};
use bande_volumes::{EjectFailure, eject, media_roots};
use bande_xdg::{
    Apps, TERMINAL, UserDir, config_dirs, config_home, data_dirs, file_uri, launch_with,
    path_from_file_uri, replace_file, spawn_detached, state_home, user_dir,
};
use rustix::event::{PollFd, PollFlags, Timespec, poll};

const APP_ID: &str = "org.bande.Files";
const WINDOW_SIZE: (u32, u32) = (1040, 620);

static INFO: Info = Info {
    id: APP_ID,
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("app-name"),
    keys: Keys::Standard,
    settings: Some(Settings {
        title: || L10N.tr("settings-title"),
        size: SETTINGS_SIZE,
        view: settings_window,
    }),
};

fn settings_window(appearance: Appearance) -> Box<dyn View> {
    Box::new(SettingsWindow::<FilesSettings>::new(
        appearance,
        settings_folder(),
        settings_view,
    ))
}

struct Pane {
    view: FilesView,
    title: String,
}

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let full_screen = window.header_height() == 0;
        window.set_frame_regions(frame_regions(full_screen));
        self.view.set_full_screen(full_screen);
        let (width, height) = window.size();
        let scene = self.view.scene(width, height);
        window.set_header(self.view.header());
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        self.view
            .set_clipboard_has_files(window.clipboard_has_uris());
        match event {
            WindowEvent::Input(input) => self.view.handle(&input),
            WindowEvent::Accessibility(request) => self.view.perform(&request),
            WindowEvent::Pasted(text) => self.view.pasted(&text),
            WindowEvent::PastedTransfer(transfer) => {
                self.view
                    .pasted_files(&transfer.uris, &transfer.text, transfer.cut)
            }
            WindowEvent::MenuAction(action) => self.view.menu_action(&action),
            WindowEvent::Drag(drag) => {
                match drag {
                    DragEvent::Entered { position, uris, .. } => {
                        let action = self.view.drag_entered(position, &uris);
                        window.accept_drag(action);
                    }
                    DragEvent::Moved(position) => {
                        let action = self.view.drag_moved(position);
                        window.accept_drag(action);
                    }
                    DragEvent::Left => self.view.drag_left(),
                    DragEvent::Dropped {
                        position,
                        transfer,
                        action,
                    } => {
                        self.view.dropped(position, &transfer.uris, action);
                    }
                    DragEvent::Finished(_) => self.view.drag_finished(),
                }
                true
            }
            WindowEvent::MenuClosed => {
                self.view.menu_closed();
                false
            }
            _ => false,
        }
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.view.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.view.set_window_active(active);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.view.fds()
    }

    fn source_ready(&mut self, _window: &mut Window, index: usize) -> bool {
        self.view.source_ready(index)
    }

    fn deadline(&self) -> Option<Instant> {
        self.view.deadline()
    }

    fn tick(&mut self, _window: &mut Window, now: Instant) -> bool {
        self.view.tick(now)
    }

    fn animating(&self) -> bool {
        self.view.animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.view.accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.view.text_input_area()
    }
}

struct Files {
    home: PathBuf,
    settings: FilesSettings,
    settings_watch: Option<SettingsWatch>,
    sidebar_watch: Option<SettingsWatch>,
    excluded: Option<Inbox<Vec<PathBuf>>>,
    search_excluded: Vec<PathBuf>,
    shows: Option<Inbox<ShowRequest>>,
    ejects: Option<Inbox<EjectFailure>>,
    recent_folders: Vec<PathBuf>,
    history: Rc<RefCell<History>>,
    trash: Trash,
}

fn bookmarks_file() -> Option<PathBuf> {
    config_home().map(|config| config.join("gtk-3.0/bookmarks"))
}

fn read(path: Option<PathBuf>) -> Option<String> {
    path.and_then(|path| std::fs::read_to_string(path).ok())
}

fn layout_file(home: &Path) -> PathBuf {
    sidebar_file(state_home().as_deref(), home)
}

struct Sidebar {
    rows: Vec<Row>,
    hidden: Vec<Location>,
    volumes_shown: bool,
}

fn recent_folders_file(home: &Path) -> PathBuf {
    recent_folders_file_in(state_home().as_deref(), home)
}

fn sidebar(home: &Path) -> Sidebar {
    let layout = read(Some(layout_file(home)));
    let rows = rows(
        home,
        read(config_home().map(|config| config.join("user-dirs.dirs"))).as_deref(),
        read(bookmarks_file()).as_deref(),
        layout.as_deref(),
    );
    Sidebar {
        rows,
        hidden: hidden_places(layout.as_deref()),
        volumes_shown: !volumes_hidden(layout.as_deref()),
    }
}

fn edit_bookmarks(edit: &BookmarkEdit) -> std::io::Result<()> {
    let file = bookmarks_file().ok_or(std::io::ErrorKind::NotFound)?;
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let edited = match edit {
        BookmarkEdit::Add(folders) => with_bookmarks(&text, folders, None),
        BookmarkEdit::Remove(folder) => without_bookmark(&text, folder),
        BookmarkEdit::Move { folder, before } => {
            with_bookmarks(&text, std::slice::from_ref(folder), before.as_deref())
        }
    };
    replace_file(&file, &edited)
}

fn edit_sidebar(home: &Path, edit: &SidebarEdit) -> std::io::Result<()> {
    if let Some(bookmarks) = &edit.bookmarks {
        edit_bookmarks(bookmarks)?;
    }
    let file = layout_file(home);
    match &edit.layout {
        Some(layout) => {
            let volumes_shown = !volumes_hidden(read(Some(file.clone())).as_deref());
            replace_file(&file, with_place_shown(layout, None, volumes_shown))
        }
        None => match std::fs::remove_file(&file) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        },
    }
}

impl Files {
    fn store(&mut self, settings: FilesSettings) {
        if let Some(folder) = settings_folder()
            && let Err(error) = settings.save(&folder)
        {
            tracing::warn!("cannot save the Files settings: {error}");
        }
        self.settings = settings;
    }

    fn follow_settings(&mut self, cx: &mut Cx<Self>) {
        if let Some(watch) = &mut self.settings_watch {
            watch.drain();
        }
        let Some(folder) = settings_folder() else {
            return;
        };
        let settings = FilesSettings::load(&folder);
        if settings == self.settings {
            return;
        }
        if settings.purge_trash && !self.settings.purge_trash {
            purge_trash(self.trash.clone());
        }
        self.settings = settings;
        for id in cx.windows().collect::<Vec<_>>() {
            if let Some((window, pane)) = cx.window(id) {
                pane.view.apply_settings(&self.settings);
                window.request_redraw();
            }
            self.changed(cx, id);
        }
    }

    fn follow_excluded(&mut self, cx: &mut Cx<Self>) {
        let Some(latest) = self.excluded.as_mut().and_then(|inbox| inbox.drain().pop()) else {
            return;
        };
        self.search_excluded = latest;
        for id in cx.windows().collect::<Vec<_>>() {
            if let Some((window, pane)) = cx.window(id) {
                pane.view.set_search_excluded(self.search_excluded.clone());
                window.request_redraw();
            }
            self.changed(cx, id);
        }
    }

    fn wait_for_show(&mut self, cx: &mut Cx<Self>) {
        let Some(inbox) = &self.shows else {
            return;
        };
        let fd = inbox.fd();
        let mut fds = [PollFd::new(&fd, PollFlags::IN)];
        let timeout = Timespec {
            tv_sec: SERVICE_WAIT.as_secs() as i64,
            tv_nsec: 0,
        };
        if poll(&mut fds, Some(&timeout)).is_ok_and(|ready| ready > 0) {
            self.follow_shows(cx);
        }
    }

    fn remember_recent_folders(&self) {
        let text: String = self
            .recent_folders
            .iter()
            .map(|folder| format!("{}\n", folder.display()))
            .collect();
        if let Err(error) = replace_file(&recent_folders_file(&self.home), &text) {
            tracing::warn!("cannot remember the recent folders: {error}");
        }
    }

    fn visited(&mut self, folder: PathBuf) {
        if folder.as_os_str().as_encoded_bytes().contains(&b'\n') {
            return;
        }
        self.recent_folders.retain(|known| *known != folder);
        self.recent_folders.insert(0, folder);
        self.recent_folders.truncate(RECENT_FOLDERS);
    }

    fn eject(&self, volume: PathBuf, force: bool) {
        let failures = self.ejects.as_ref().map(Inbox::outbox);
        eject(vec![volume], force, move |failure| {
            if let Some(failures) = &failures {
                failures.send(failure);
            }
        });
    }

    fn follow_ejects(&mut self, cx: &mut Cx<Self>) {
        let failures = self.ejects.as_mut().map(Inbox::drain).unwrap_or_default();
        let shown = cx.focused().or_else(|| cx.windows().next());
        for failure in failures {
            if let Some((window, pane)) = shown.and_then(|id| cx.window(id)) {
                pane.view.present_eject_failure(failure);
                window.request_redraw();
            }
        }
    }

    fn follow_shows(&mut self, cx: &mut Cx<Self>) {
        let requests = self.shows.as_mut().map(Inbox::drain).unwrap_or_default();
        for request in requests {
            let mut opened = None;
            for arguments in show_arguments(&request) {
                match self.open_window(cx, &arguments, None) {
                    Ok(window) => opened = Some(window),
                    Err(error) => tracing::warn!("cannot open a window: {error}"),
                }
            }
            if let Some(window) = opened {
                cx.raise(window, request.token);
            }
        }
    }

    fn load_sidebars(&mut self, cx: &mut Cx<Self>) {
        if let Some(watch) = &mut self.sidebar_watch {
            watch.drain();
        }
        let sidebar = sidebar(&self.home);
        for id in cx.windows().collect::<Vec<_>>() {
            if let Some((window, pane)) = cx.window(id) {
                pane.view.set_sidebar(sidebar.rows.clone());
                pane.view
                    .set_hidden_places(sidebar.hidden.clone(), sidebar.volumes_shown);
                window.request_redraw();
            }
        }
    }

    fn open_window(
        &self,
        cx: &mut Cx<Self>,
        arguments: &[String],
        directory: Option<&Path>,
    ) -> Result<WindowId, bande_app::Error> {
        let info = arguments.first().is_some_and(|flag| flag == INFO_FLAG);
        let select = info || arguments.first().is_some_and(|flag| flag == SELECT_FLAG);
        let user_dirs = config_home()
            .and_then(|config| std::fs::read_to_string(config.join("user-dirs.dirs")).ok());
        let mut targets = arguments
            .iter()
            .skip(usize::from(select))
            .filter(|argument| *argument != NEW_WINDOW_FLAG)
            .map(String::as_str);
        let location = start_location(
            targets.next(),
            self.settings.new_window.location(
                &self.home,
                user_dirs.as_deref(),
                self.recent_folders.first().map(PathBuf::as_path),
                self.settings.new_window_folder.as_deref(),
            ),
            directory,
        );
        let reveal = match &location {
            Location::Folder(path) if select || !path.is_dir() => {
                let mut paths = vec![path.clone()];
                paths.extend(targets.filter_map(|target| {
                    match start_location(Some(target), Location::Recents, directory) {
                        Location::Folder(other) if other.parent() == path.parent() => Some(other),
                        _ => None,
                    }
                }));
                Some(paths)
            }
            _ => None,
        };
        let mut browser = Browser::open(
            match reveal.as_ref().and_then(|paths| paths.first()) {
                Some(path) => Location::Folder(
                    path.parent()
                        .map_or_else(|| self.home.clone(), PathBuf::from),
                ),
                None => location,
            },
            self.trash.clone(),
        );
        if let Some(paths) = reveal {
            browser.reveal_all(&paths);
        }
        let sidebar = sidebar(&self.home);
        let apps = Apps::load(data_dirs(), &config_dirs());
        let thumbnails = Thumbnails::for_user(apps.data_dirs());
        let mut view = FilesView::new(
            browser,
            sidebar.rows,
            apps,
            Appearance::default().palette(),
            TextSystem::new(),
        )?;
        view.set_hidden_places(sidebar.hidden, sidebar.volumes_shown);
        view.share_history(Rc::clone(&self.history));
        view.set_default_view(self.settings.default_view);
        view.apply_settings(&self.settings);
        if let Some(thumbnails) = thumbnails {
            view.set_thumbnails(thumbnails);
        }
        view.set_templates(user_dir(
            &self.home,
            user_dirs.as_deref(),
            UserDir::Templates,
        ));
        let state_home = state_home();
        view.set_view_memory(ViewMemory::load(views_file(
            state_home.as_deref(),
            &self.home,
        )));
        view.set_status_bar_file(status_bar_file(state_home.as_deref(), &self.home));
        view.set_list_columns_file(list_columns_file(state_home.as_deref(), &self.home));
        let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
        if let Some(usage) = usage_path(state_home.as_deref(), Some(&self.home)) {
            view.set_recents_sources(recently_used_path(data_home.as_deref(), &self.home), usage);
        }
        view.set_search_excluded(self.search_excluded.clone());
        if info {
            view.show_inspector();
        }
        view.set_media(media_roots(&std::env::var("USER").unwrap_or_default()));
        let title = view.title();
        let window = cx.open(Kind::Main, &title.clone(), WINDOW_SIZE, |appearance| {
            view.set_appearance(appearance);
            Pane { view, title }
        })?;
        Ok(window)
    }

    fn serve(&mut self, cx: &mut Cx<Self>, id: WindowId, request: Request) {
        if let Request::NewWindow = request {
            if let Err(error) = self.open_window(cx, &[], None) {
                tracing::warn!("cannot open a window: {error}");
            }
            return;
        }
        let Some((window, pane)) = cx.window(id) else {
            return;
        };
        match request {
            Request::Open { app, files } => {
                let token = window.activation_token(app.app_id()).ok();
                if let Err(error) = launch_with(&app, &files, token.as_deref()) {
                    tracing::warn!("cannot open with {}: {error}", app.id);
                }
            }
            Request::CopyFiles { paths, cut } => {
                let uris = paths.iter().map(|path| file_uri(path)).collect();
                let transfer = Transfer::uris(uris);
                window.copy_transfer(if cut { transfer.cut() } else { transfer });
            }
            Request::StartDrag { paths, grab } => {
                let icon = pane.view.drag_icon(grab);
                let uris = paths.iter().map(|path| file_uri(path)).collect();
                let actions = [DragAction::Copy, DragAction::Move];
                if let Err(error) = window.start_drag(Transfer::uris(uris), &actions, Some(icon)) {
                    tracing::warn!("cannot start dragging: {error}");
                    pane.view.drag_finished();
                }
            }
            Request::PasteFiles => {
                if let Err(error) = window.paste_transfer() {
                    tracing::warn!("cannot paste: {error}");
                }
            }
            Request::Clipboard(request) => {
                if let Err(error) = window.serve_clipboard(request) {
                    tracing::warn!("cannot use the clipboard: {error}");
                }
            }
            Request::ShowMenu { menu, at } => {
                if let Err(error) = window.show_menu(menu, MenuPlacement::AtPoint(at)) {
                    tracing::warn!("cannot show the menu: {error}");
                }
            }
            Request::Eject(volume) => self.eject(volume, false),
            Request::ForceEject(volume) => self.eject(volume, true),
            Request::NewWindow => {}
            Request::OpenTerminal(folder) => open_terminal(window, &folder),
            Request::EditSidebar(edit) => {
                if let Err(error) = edit_sidebar(&self.home, &edit) {
                    tracing::warn!("cannot change the sidebar: {error}");
                }
                self.load_sidebars(cx);
            }
            Request::ShowEmojiPicker => {
                if let Err(error) = window.show_emoji_picker() {
                    tracing::warn!("cannot show the emoji picker: {error}");
                }
            }
            Request::PlaySound(name) => play_sound(name),
            Request::ShowHidden(show) => self.store(FilesSettings {
                show_hidden: show,
                ..self.settings.clone()
            }),
        }
    }
}

impl Program for Files {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
        let trash = Trash::for_user().ok_or("no home folder for the Trash")?;
        let folder = settings_folder();
        let settings = folder
            .as_deref()
            .map(FilesSettings::load)
            .unwrap_or_default();
        let settings_watch = folder.as_deref().and_then(|folder| {
            purge_trash_daily(trash.clone(), folder.to_path_buf());
            SettingsWatch::start(folder, FilesSettings::FILE)
        });
        let excluded = Inbox::new()
            .inspect_err(|error| tracing::warn!("cannot follow the Search privacy list: {error}"))
            .ok();
        if let Some(inbox) = &excluded {
            follow_search_excluded(home.clone(), inbox.outbox());
        }
        let sidebar_watch = layout_file(&home)
            .parent()
            .and_then(|folder| SettingsWatch::start(folder, SIDEBAR_FILE));
        let shows = Inbox::new()
            .inspect_err(|error| tracing::warn!("cannot show folders for other apps: {error}"))
            .ok();
        if let Some((session, inbox)) = cx.session().zip(shows.as_ref())
            && let Err(error) = serve_file_manager(session, inbox.outbox())
        {
            tracing::warn!("cannot serve org.freedesktop.FileManager1: {error}");
        }
        let recent_folders = read(Some(recent_folders_file(&home)))
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .collect();
        let mut files = Self {
            home,
            settings,
            settings_watch,
            sidebar_watch,
            excluded,
            search_excluded: Vec::new(),
            shows,
            ejects: Inbox::new()
                .inspect_err(|error| tracing::warn!("cannot report eject failures: {error}"))
                .ok(),
            recent_folders,
            history: Rc::default(),
            trash,
        };
        match arguments.iter().any(|argument| argument == SERVICE_FLAG) {
            true => files.wait_for_show(cx),
            false => {
                let directory = std::env::current_dir().ok();
                files.open_window(cx, &arguments, directory.as_deref())?;
            }
        }
        Ok(files)
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        cx.focused()
            .and_then(|id| cx.view(id))
            .or_else(|| cx.windows().last().and_then(|id| cx.view(id)))
            .map_or_else(menu_bar::windowless_menus, |pane| {
                menu_bar::app_menus(&pane.view, &self.recent_folders)
            })
    }

    fn command(&mut self, cx: &mut Cx<Self>, window: Option<WindowId>, command: &str) {
        if command == menu_bar::CLEAR_RECENT_FOLDERS {
            self.recent_folders.clear();
            self.remember_recent_folders();
            cx.refresh_menus();
            return;
        }
        let Some(window) = window.filter(|window| cx.view(*window).is_some()) else {
            if command == menu_bar::NEW_WINDOW {
                self.reopen(cx);
            }
            return;
        };
        let recent = command
            .strip_prefix(menu_bar::RECENT_FOLDER)
            .and_then(|index| index.parse::<usize>().ok())
            .and_then(|index| self.recent_folders.get(index).cloned());
        let Some((shown, pane)) = cx.window(window) else {
            return;
        };
        if let Some(folder) = recent {
            pane.view.go(Location::Folder(folder));
            shown.request_redraw();
            self.changed(cx, window);
            return;
        }
        pane.view
            .set_clipboard_has_files(shown.clipboard_has_uris());
        if pane.view.menu_action(command) {
            shown.request_redraw();
            self.changed(cx, window);
        }
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, directory: &Path) {
        if arguments.iter().any(|argument| argument == SERVICE_FLAG) {
            return;
        }
        if let Err(error) = self.open_window(cx, &arguments, Some(directory)) {
            tracing::warn!("cannot open a window: {error}");
        }
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        if let Err(error) = self.open_window(cx, &[], None) {
            tracing::warn!("cannot open a window: {error}");
        }
    }

    fn changed(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        let requests = cx
            .window(window)
            .map(|(_, pane)| pane.view.take_requests())
            .unwrap_or_default();
        for request in requests {
            self.serve(cx, window, request);
        }
        if let Some((shown, pane)) = cx.window(window) {
            let title = pane.view.title();
            if title != pane.title {
                shown.set_title(&title);
                pane.title = title;
                if let Some(folder) = pane.view.browser().current_folder() {
                    self.visited(folder);
                }
            }
            shown.set_cursor(pane.view.cursor());
            shown.request_redraw();
        }
        cx.refresh_menus();
    }

    fn window_closed(&mut self, _cx: &mut Cx<Self>, _window: WindowId, _view: Pane) {
        self.remember_recent_folders();
    }

    fn should_close(&mut self, _cx: &mut Cx<Self>, _window: WindowId) -> bool {
        self.remember_recent_folders();
        true
    }

    fn close_tab(&mut self, cx: &mut Cx<Self>, window: WindowId) -> bool {
        let closed = cx
            .window(window)
            .is_some_and(|(_, pane)| pane.view.close_tab(pane.view.active_tab()));
        if closed {
            self.changed(cx, window);
        }
        closed
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.settings_watch
            .iter()
            .map(SettingsWatch::fd)
            .chain(self.sidebar_watch.iter().map(SettingsWatch::fd))
            .chain(self.excluded.iter().map(Inbox::fd))
            .chain(self.shows.iter().map(Inbox::fd))
            .chain(self.ejects.iter().map(Inbox::fd))
            .collect()
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, index: usize) {
        let settings = self.settings_watch.iter().count();
        let sidebar = settings + self.sidebar_watch.iter().count();
        let excluded = sidebar + self.excluded.iter().count();
        let shows = excluded + self.shows.iter().count();
        if index < settings {
            self.follow_settings(cx);
        } else if index < sidebar {
            self.load_sidebars(cx);
        } else if index < excluded {
            self.follow_excluded(cx);
        } else if index < shows {
            self.follow_shows(cx);
        } else {
            self.follow_ejects(cx);
        }
    }
}

const INFO_FLAG: &str = "--info";
const NEW_WINDOW_FLAG: &str = "--new-window";
const SELECT_FLAG: &str = "--select";
const RECENT_FOLDERS: usize = 10;
const SERVICE_FLAG: &str = "--gapplication-service";
const SERVICE_WAIT: Duration = Duration::from_secs(5);
const PURGE_AFTER_DAYS: i64 = 30;
const PURGE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const TERMINAL_APP_ID: &str = "org.bande.Terminal";
const TERMINAL_DIRECTORY: &str = "--working-directory";
const TRASH_URI: &str = "trash:///";
const RECENTS_URI: &str = "recent:///";

fn frame_regions(full_screen: bool) -> FrameRegions {
    FrameRegions {
        sidebar: SIDEBAR_WIDTH,
        toolbar: if full_screen { TOOLBAR_HEIGHT } else { 0.0 },
    }
}

fn play_sound(name: &'static str) {
    let played = std::thread::Builder::new()
        .name("sound".to_owned())
        .spawn(move || match zbus::blocking::Connection::session() {
            Ok(session) => bande_sound::play(&session, name),
            Err(error) => tracing::debug!("cannot play {name}: {error}"),
        });
    if let Err(error) = played {
        tracing::warn!("cannot play {name}: {error}");
    }
}

fn open_terminal(window: &mut Window, folder: &Path) {
    let token = window.activation_token(TERMINAL_APP_ID).ok();
    let command = [
        TERMINAL.to_owned(),
        TERMINAL_DIRECTORY.to_owned(),
        folder.to_string_lossy().into_owned(),
    ];
    if let Err(error) = spawn_detached(TERMINAL_APP_ID, &command, None, token.as_deref()) {
        tracing::warn!("cannot open a terminal: {error}");
    }
}

fn purge_trash(trash: Trash) {
    let spawned = std::thread::Builder::new()
        .name("purge".to_owned())
        .spawn(move || purge_old_items(&trash));
    if let Err(error) = spawned {
        tracing::warn!("cannot empty old items from the Trash: {error}");
    }
}

fn purge_old_items(trash: &Trash) {
    let before = jiff::Zoned::now()
        .datetime()
        .checked_sub(jiff::Span::new().days(PURGE_AFTER_DAYS));
    let purged = match before {
        Ok(before) => trash.purge(before),
        Err(error) => return tracing::warn!("cannot date the Trash: {error}"),
    };
    if let Err(error) = purged {
        tracing::warn!("cannot remove old items from the Trash: {error}");
    }
}

fn purge_trash_daily(trash: Trash, folder: PathBuf) {
    let spawned = std::thread::Builder::new()
        .name("purge".to_owned())
        .spawn(move || {
            loop {
                if FilesSettings::load(&folder).purge_trash {
                    purge_old_items(&trash);
                }
                std::thread::sleep(PURGE_INTERVAL);
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("cannot empty old items from the Trash: {error}");
    }
}

fn show_arguments(request: &ShowRequest) -> Vec<Vec<String>> {
    match request.show {
        Show::Folders => request.uris.iter().map(|uri| vec![uri.clone()]).collect(),
        Show::Properties => request
            .uris
            .iter()
            .map(|uri| vec![INFO_FLAG.to_owned(), uri.clone()])
            .collect(),
        Show::Items => {
            let mut groups: Vec<(Option<PathBuf>, Vec<String>)> = Vec::new();
            for uri in &request.uris {
                let parent =
                    path_from_file_uri(uri).and_then(|path| path.parent().map(PathBuf::from));
                match groups.iter_mut().find(|(known, _)| *known == parent) {
                    Some((_, group)) => group.push(uri.clone()),
                    None => groups.push((parent, vec![SELECT_FLAG.to_owned(), uri.clone()])),
                }
            }
            groups.into_iter().map(|(_, group)| group).collect()
        }
    }
}

fn start_location(
    argument: Option<&str>,
    fallback: Location,
    directory: Option<&Path>,
) -> Location {
    let Some(argument) = argument else {
        return fallback;
    };
    if argument == TRASH_URI || argument == "trash:" {
        return Location::Trash;
    }
    if argument == RECENTS_URI {
        return Location::Recents;
    }
    let path = path_from_file_uri(argument).unwrap_or_else(|| PathBuf::from(argument));
    Location::Folder(directory.map_or(path.clone(), |directory| directory.join(&path)))
}

fn main() -> ExitCode {
    bande_app::run::<Files>(&INFO)
}
