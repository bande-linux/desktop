mod menu_bar;

use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_app_menu::{AppMenu, Standard};
use bande_files::{
    BookmarkEdit, Browser, FilesView, Location, Request, Row, SIDEBAR_WIDTH, SidebarEdit,
    TOOLBAR_HEIGHT, ViewMemory, hidden_places, list_columns_file, rows, sidebar_file,
    status_bar_file, views_file, with_bookmarks, without_bookmark,
};
use bande_fs::Trash;
use bande_search::{recently_used_path, usage_path};
use bande_thumbnails::Thumbnails;
use bande_toolkit::ui::shortcut_pressed;
use bande_toolkit::{
    App, Appearance, DragAction, DragEvent, FrameRegions, Header, MenuPlacement, Rect, Role, Scene,
    TextSystem, Transfer, Window, WindowEvent,
};
use bande_volumes::{eject, media_roots};
use bande_xdg::{
    Apps, TERMINAL, UserDir, config_dirs, config_home, data_dirs, file_uri, launch_with,
    path_from_file_uri, spawn_detached, state_home, user_dir,
};

const APP_ID: &str = "org.bande.Files";
const WINDOW_SIZE: (u32, u32) = (1040, 620);

struct Files {
    view: FilesView,
    title: String,
    home: PathBuf,
    app_menu: Option<AppMenu>,
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

fn sidebar(home: &Path) -> (Vec<Row>, Vec<Location>) {
    let layout = read(Some(layout_file(home)));
    let rows = rows(
        home,
        read(config_home().map(|config| config.join("user-dirs.dirs"))).as_deref(),
        read(bookmarks_file()).as_deref(),
        layout.as_deref(),
    );
    (rows, hidden_places(layout.as_deref()))
}

fn replace(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let staging = file.with_extension("new");
    std::fs::write(&staging, text)?;
    std::fs::rename(staging, file)
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
    replace(&file, &edited)
}

fn edit_sidebar(home: &Path, edit: &SidebarEdit) -> std::io::Result<()> {
    if let Some(bookmarks) = &edit.bookmarks {
        edit_bookmarks(bookmarks)?;
    }
    let file = layout_file(home);
    match &edit.layout {
        Some(layout) => replace(&file, layout),
        None => match std::fs::remove_file(&file) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        },
    }
}

impl Files {
    fn load_sidebar(&mut self) {
        let (rows, hidden) = sidebar(&self.home);
        self.view.set_sidebar(rows);
        self.view.set_hidden_places(hidden);
    }

    fn menu_bar_action(&mut self, window: &mut Window, action: &str) -> bool {
        self.view
            .set_clipboard_has_files(window.clipboard_has_uris());
        let result = match Standard::parse(action) {
            Some(Standard::Hide | Standard::Minimize) => window.minimize(),
            Some(Standard::Zoom) => window.toggle_zoom(),
            Some(Standard::Quit) => {
                window.close();
                Ok(())
            }
            Some(Standard::CloseWindow) => {
                if !self.view.close_tab(self.view.active_tab()) {
                    window.close();
                }
                return true;
            }
            None => {
                return match menu_bar::key_event(action) {
                    Some(event) => self.view.handle(&event),
                    None => self.view.menu_action(action),
                };
            }
        };
        if let Err(error) = result {
            tracing::warn!("cannot {action}: {error}");
        }
        false
    }

    fn changed(&mut self, window: &mut Window) {
        for request in self.view.take_requests() {
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
                    let icon = self.view.drag_icon(grab);
                    let uris = paths.iter().map(|path| file_uri(path)).collect();
                    let actions = [DragAction::Copy, DragAction::Move];
                    if let Err(error) =
                        window.start_drag(Transfer::uris(uris), &actions, Some(icon))
                    {
                        tracing::warn!("cannot start dragging: {error}");
                        self.view.drag_finished();
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
                Request::Eject(volume) => eject(vec![volume]),
                Request::NewWindow(location) => new_window(window, &location),
                Request::OpenTerminal(folder) => open_terminal(window, &folder),
                Request::EditSidebar(edit) => {
                    if let Err(error) = edit_sidebar(&self.home, &edit) {
                        tracing::warn!("cannot change the sidebar: {error}");
                    }
                    self.load_sidebar();
                }
                Request::ShowEmojiPicker => {
                    if let Err(error) = window.show_emoji_picker() {
                        tracing::warn!("cannot show the emoji picker: {error}");
                    }
                }
                Request::PlaySound(name) => play_sound(name),
            }
        }
        let title = self.view.title();
        if title != self.title {
            window.set_title(&title);
            self.title = title;
        }
        if let Some(app_menu) = &self.app_menu
            && let Err(error) = app_menu.update(menu_bar::app_menus(&self.view))
        {
            tracing::warn!("cannot update the menus: {error}");
        }
        window.set_cursor(self.view.cursor());
        window.request_redraw();
    }
}

impl App for Files {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let full_screen = window.header_height() == 0;
        window.set_frame_regions(frame_regions(full_screen));
        self.view.set_full_screen(full_screen);
        let (width, height) = window.size();
        let scene = self.view.scene(width, height);
        window.set_header(self.view.header());
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        self.view
            .set_clipboard_has_files(window.clipboard_has_uris());
        let changed = match event {
            WindowEvent::AppearanceChanged(appearance) => {
                self.view.set_appearance(appearance);
                true
            }
            WindowEvent::Input(input) => {
                match Standard::for_shortcut(|shortcut| shortcut_pressed(&input, shortcut)) {
                    Some(standard) => self.menu_bar_action(window, standard.action()),
                    None => self.view.handle(&input),
                }
            }
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
            WindowEvent::Focus(focused) => {
                if focused && let Some(Err(error)) = self.app_menu.as_ref().map(AppMenu::claim) {
                    tracing::warn!("{error}");
                }
                self.view.set_window_active(focused)
            }
            WindowEvent::MenuClosed => {
                self.view.menu_closed();
                false
            }
            _ => false,
        };
        if changed {
            self.changed(window);
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut sources = self.view.fds();
        sources.extend(self.app_menu.as_ref().map(AppMenu::fd));
        sources
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        if index == self.view.fds().len() {
            let actions = self
                .app_menu
                .as_mut()
                .map(AppMenu::take_actions)
                .unwrap_or_default();
            let mut changed = false;
            for action in actions {
                changed |= self.menu_bar_action(window, &action);
            }
            if changed {
                self.changed(window);
            }
        } else if self.view.source_ready(index) {
            self.changed(window);
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.view.deadline()
    }

    fn animating(&self) -> bool {
        self.view.animating()
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.view.text_input_area()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.view.accessibility())
    }

    fn should_close(&mut self, window: &mut Window) -> bool {
        if window.close_requested_by_button() || !self.view.close_tab(self.view.active_tab()) {
            return true;
        }
        self.changed(window);
        false
    }

    fn should_quit(&mut self, _window: &mut Window) -> bool {
        true
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if self.view.tick(now) {
            self.changed(window);
        }
    }
}

const INFO_FLAG: &str = "--info";
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

fn new_window(window: &mut Window, location: &Location) {
    let program = match std::env::current_exe() {
        Ok(program) => program.to_string_lossy().into_owned(),
        Err(error) => return tracing::warn!("cannot find the Files program: {error}"),
    };
    let argument = match location {
        Location::Trash => TRASH_URI.to_owned(),
        Location::Recents => RECENTS_URI.to_owned(),
        Location::Folder(folder) | Location::Search(folder) => {
            folder.to_string_lossy().into_owned()
        }
    };
    let token = window.activation_token(APP_ID).ok();
    if let Err(error) = spawn_detached(APP_ID, &[program, argument], None, token.as_deref()) {
        tracing::warn!("cannot open a window: {error}");
    }
}

fn start_location(home: &Path, argument: Option<&str>) -> Location {
    let Some(argument) = argument else {
        return Location::Folder(home.to_path_buf());
    };
    if argument == TRASH_URI || argument == "trash:" {
        return Location::Trash;
    }
    if argument == RECENTS_URI {
        return Location::Recents;
    }
    let path = path_from_file_uri(argument).unwrap_or_else(|| PathBuf::from(argument));
    Location::Folder(std::path::absolute(&path).unwrap_or(path))
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
    let trash = Trash::for_user().ok_or("no home folder for the Trash")?;
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let info = arguments.first().is_some_and(|flag| flag == INFO_FLAG);
    let location = start_location(&home, arguments.get(usize::from(info)).map(String::as_str));
    let reveal = match &location {
        Location::Folder(path) if info || !path.is_dir() => Some(path.clone()),
        _ => None,
    };
    let mut browser = Browser::open(
        match &reveal {
            Some(path) => {
                Location::Folder(path.parent().map_or_else(|| home.clone(), PathBuf::from))
            }
            None => location,
        },
        trash,
    );
    if let Some(path) = reveal {
        browser.reveal(&path);
    }
    let (sidebar, hidden) = sidebar(&home);
    let apps = Apps::load(data_dirs(), &config_dirs());
    let thumbnails = Thumbnails::for_user(apps.data_dirs());
    let mut view = FilesView::new(
        browser,
        sidebar,
        apps,
        Appearance::default().palette(),
        TextSystem::new(),
    )?;
    view.set_hidden_places(hidden);
    if let Some(thumbnails) = thumbnails {
        view.set_thumbnails(thumbnails);
    }
    let user_dirs = config_home()
        .and_then(|config| std::fs::read_to_string(config.join("user-dirs.dirs")).ok());
    view.set_templates(user_dir(&home, user_dirs.as_deref(), UserDir::Templates));
    let state_home = state_home();
    view.set_view_memory(ViewMemory::load(views_file(state_home.as_deref(), &home)));
    view.set_status_bar_file(status_bar_file(state_home.as_deref(), &home));
    view.set_list_columns_file(list_columns_file(state_home.as_deref(), &home));
    let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    if let Some(usage) = usage_path(state_home.as_deref(), Some(&home)) {
        view.set_recents_sources(recently_used_path(data_home.as_deref(), &home), usage);
    }
    if info {
        view.show_inspector();
    }
    view.set_media(media_roots(&std::env::var("USER").unwrap_or_default()));
    let title = view.title();
    let mut window = Window::open(Role::toplevel(&title, APP_ID), WINDOW_SIZE)?;
    window.set_frame_regions(frame_regions(false));
    window.set_header(Header::default());
    let app_menu = AppMenu::export(APP_ID, menu_bar::app_menus(&view))
        .inspect_err(|error| tracing::warn!("{error}"))
        .ok();
    let mut files = Files {
        view,
        title,
        home,
        app_menu,
    };
    bande_toolkit::run(&mut window, &mut files)?;
    Ok(())
}

fn main() -> ExitCode {
    bande_log::init("bande-files");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
