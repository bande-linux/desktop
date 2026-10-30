mod menu_bar;

use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use accesskit::TreeUpdate;
use unbound_app_menu::{AppMenu, Standard};
use unbound_files::{Browser, FilesView, Location, Request, Row, rows, without_bookmark};
use unbound_fs::Trash;
use unbound_toolkit::{
    App, Appearance, DragAction, DragEvent, MenuPlacement, Rect, Role, Scene, TextSystem, Transfer,
    Window, WindowEvent,
};
use unbound_volumes::{eject, media_roots};
use unbound_xdg::{
    Apps, config_dirs, config_home, data_dirs, file_uri, launch_with, path_from_file_uri,
};

const APP_ID: &str = "org.unbound.Files";
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

fn sidebar(home: &Path) -> Vec<Row> {
    let read = |path: Option<PathBuf>| path.and_then(|path| std::fs::read_to_string(path).ok());
    rows(
        home,
        read(config_home().map(|config| config.join("user-dirs.dirs"))).as_deref(),
        read(bookmarks_file()).as_deref(),
    )
}

fn remove_bookmark(folder: &Path) -> std::io::Result<()> {
    let file = bookmarks_file().ok_or(std::io::ErrorKind::NotFound)?;
    let text = std::fs::read_to_string(&file)?;
    let staging = file.with_extension("new");
    std::fs::write(&staging, without_bookmark(&text, folder))?;
    std::fs::rename(staging, file)
}

impl Files {
    fn menu_bar_action(&mut self, window: &mut Window, action: &str) -> bool {
        self.view
            .set_clipboard_has_files(window.clipboard_has_uris());
        let result = match Standard::parse(action) {
            Some(Standard::Hide | Standard::Minimize) => window.minimize(),
            Some(Standard::Zoom) => window.toggle_zoom(),
            Some(Standard::Quit | Standard::CloseWindow) => {
                window.close();
                Ok(())
            }
            None => {
                return menu_bar::key_event(action).is_some_and(|event| self.view.handle(&event));
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
                Request::CopyText(text) => window.copy(&text),
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
                Request::Paste => {
                    if let Err(error) = window.paste() {
                        tracing::warn!("cannot paste: {error}");
                    }
                }
                Request::ShowMenu { menu, at } => {
                    if let Err(error) = window.show_menu(menu, MenuPlacement::AtPoint(at)) {
                        tracing::warn!("cannot show the menu: {error}");
                    }
                }
                Request::Eject(volume) => eject(vec![volume]),
                Request::RemoveBookmark(folder) => match remove_bookmark(&folder) {
                    Ok(()) => self.view.set_sidebar(sidebar(&self.home)),
                    Err(error) => tracing::warn!("cannot remove the bookmark: {error}"),
                },
            }
        }
        let title = self.view.title();
        if title != self.title {
            window.set_title(&title);
            self.title = title;
        }
        window.set_cursor(self.view.cursor());
        window.request_redraw();
    }
}

impl App for Files {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.view.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        self.view
            .set_clipboard_has_files(window.clipboard_has_uris());
        let changed = match event {
            WindowEvent::AppearanceChanged(appearance) => {
                self.view.set_palette(appearance.palette());
                true
            }
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
            WindowEvent::Focus(true) => {
                if let Some(Err(error)) = self.app_menu.as_ref().map(AppMenu::claim) {
                    tracing::warn!("{error}");
                }
                false
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

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if self.view.tick(now) {
            self.changed(window);
        }
    }
}

const INFO_FLAG: &str = "--info";

fn start_location(home: &Path, argument: Option<&str>) -> Location {
    let Some(argument) = argument else {
        return Location::Folder(home.to_path_buf());
    };
    if argument == "trash:///" || argument == "trash:" {
        return Location::Trash;
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
    let sidebar = sidebar(&home);
    let apps = Apps::load(data_dirs(), &config_dirs());
    let mut view = FilesView::new(
        browser,
        sidebar,
        apps,
        Appearance::default().palette(),
        TextSystem::new(),
    )?;
    if info {
        view.show_inspector();
    }
    view.set_media(media_roots(&std::env::var("USER").unwrap_or_default()));
    let title = view.title();
    let mut window = Window::open(Role::toplevel(&title, APP_ID), WINDOW_SIZE)?;
    let app_menu = AppMenu::export(APP_ID, menu_bar::app_menus())
        .inspect_err(|error| tracing::warn!("{error}"))
        .ok();
    let mut files = Files {
        view,
        title,
        home,
        app_menu,
    };
    unbound_toolkit::run(&mut window, &mut files)?;
    Ok(())
}

fn main() -> ExitCode {
    unbound_log::init("unbound-files");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
