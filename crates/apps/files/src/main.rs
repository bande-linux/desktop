use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use unbound_files::{Browser, FilesView, Location, Request, rows};
use unbound_fs::Trash;
use unbound_toolkit::{App, Appearance, Role, Scene, TextSystem, Window, WindowEvent};
use unbound_xdg::{Apps, config_dirs, config_home, data_dirs, launch_with, path_from_file_uri};

const APP_ID: &str = "org.unbound.Files";
const WINDOW_SIZE: (u32, u32) = (1040, 620);

struct Files {
    view: FilesView,
    title: String,
}

impl Files {
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
                Request::Paste => {
                    if let Err(error) = window.paste() {
                        tracing::warn!("cannot paste: {error}");
                    }
                }
            }
        }
        let title = self.view.title();
        if title != self.title {
            window.set_title(&title);
            self.title = title;
        }
        window.request_redraw();
    }
}

impl App for Files {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.view.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let changed = match event {
            WindowEvent::AppearanceChanged(appearance) => {
                self.view.set_palette(appearance.palette());
                true
            }
            WindowEvent::Input(input) => self.view.handle(&input),
            WindowEvent::Pasted(text) => self.view.pasted(&text),
            _ => false,
        };
        if changed {
            self.changed(window);
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.view.fds()
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        if self.view.source_ready(index) {
            self.changed(window);
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.view.deadline()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if self.view.tick(now) {
            self.changed(window);
        }
    }
}

fn start_location(home: &Path) -> Location {
    let Some(argument) = std::env::args().nth(1) else {
        return Location::Folder(home.to_path_buf());
    };
    if argument == "trash:///" || argument == "trash:" {
        return Location::Trash;
    }
    let path = path_from_file_uri(&argument).unwrap_or_else(|| PathBuf::from(&argument));
    Location::Folder(std::path::absolute(&path).unwrap_or(path))
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
    let trash = Trash::for_user().ok_or("no home folder for the Trash")?;
    let location = start_location(&home);
    let reveal = match &location {
        Location::Folder(path) if !path.is_dir() => Some(path.clone()),
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
    let config = config_home();
    let read = |path: Option<PathBuf>| path.and_then(|path| std::fs::read_to_string(path).ok());
    let sidebar = rows(
        &home,
        read(config.as_ref().map(|config| config.join("user-dirs.dirs"))).as_deref(),
        read(
            config
                .as_ref()
                .map(|config| config.join("gtk-3.0/bookmarks")),
        )
        .as_deref(),
    );
    let apps = Apps::load(data_dirs(), &config_dirs());
    let view = FilesView::new(
        browser,
        sidebar,
        apps,
        Appearance::default().palette(),
        TextSystem::new(),
    )?;
    let title = view.title();
    let mut window = Window::open(Role::toplevel(&title, APP_ID), WINDOW_SIZE)?;
    let mut files = Files { view, title };
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
