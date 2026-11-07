mod access;
mod account;
mod app_chooser;
mod launcher;
mod print;

use std::cell::Cell;
use std::ffi::OsString;
use std::io::Write;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_app_settings::{Stored, settings_folder};
use bande_content::pdf::protocol::WORKER_FLAG;
use bande_file_chooser::{
    Chooser, ChooserView, Finished, Icons, L10N, initial_folder, remember, remembered_folder,
};
use bande_files::{FilesSettings, rows, sidebar_file, with_volumes};
use bande_toolkit::ui::Alert;
use bande_toolkit::{App, Image, Rect, Role, Scene, TextSystem, Window, WindowEvent};
use bande_volumes::{media_roots, mounted_volumes, volume_icon};
use bande_wallpaper::Fit;
use bande_xdg::{
    Locale, MimeDatabase, UserDir, app_name, applications, config_home, current_desktops,
    data_dirs, replace_file, state_home, user_dir,
};
use xdg_desktop_portal_bande::dialog_process::{
    ACCESS, CANCELLED, CHOOSE_APP, CHOSEN, CREATE_LAUNCHER, PRINT, SET_WALLPAPER, SHARE_ACCOUNT,
    encode, parse, parse_access, parse_account, parse_app_chooser, parse_launcher,
};
use xdg_desktop_portal_bande::file_chooser::{Dialog, Mode, Selection};

const APP_ID: &str = "org.bande.FileChooser";
const WINDOW_SIZE: (u32, u32) = (820, 520);
const UNAVAILABLE: u8 = 2;
const PREVIEW_SIZE: f64 = 36.0;
const SET: &str = "set";
const WAYLAND_PARENT: &str = "wayland:";

struct FileDialog {
    view: ChooserView,
    chosen: Option<Finished>,
}

impl App for FileDialog {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.view.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let redraw = match event {
            WindowEvent::Input(input) => {
                let handled = self.view.handle(&input);
                window.set_cursor(self.view.cursor());
                handled
            }
            WindowEvent::Accessibility(request) => self.view.perform(&request),
            WindowEvent::AppearanceChanged(appearance) => {
                self.view.set_appearance(appearance);
                true
            }
            WindowEvent::Focus(focused) => self.view.set_window_active(focused),
            _ => false,
        };
        if redraw {
            window.request_redraw();
        }
        if let Some(finished) = self.view.take_finished() {
            self.chosen = Some(finished);
            window.close();
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.view.listing_fd().into_iter().collect()
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) {
        self.view.listing_ready();
        window.request_redraw();
    }

    fn deadline(&self) -> Option<Instant> {
        self.view.deadline()
    }

    fn tick(&mut self, window: &mut Window, _now: Instant) {
        window.request_redraw();
    }

    fn finished(&self) -> bool {
        self.chosen.is_some()
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
}

fn memory_file() -> Option<PathBuf> {
    bande_xdg::state_home().map(|state| state.join("bande/file-chooser-folders"))
}

fn read(path: Option<PathBuf>) -> Option<String> {
    path.and_then(|path| std::fs::read_to_string(path).ok())
}

fn remember_folder(app_id: &str, folder: &Path) {
    let Some(file) = memory_file() else {
        return;
    };
    let updated = remember(
        &read(Some(file.clone())).unwrap_or_default(),
        app_id,
        folder,
    );
    if let Err(error) = replace_file(&file, &updated) {
        tracing::warn!("cannot remember the folder: {error}");
    }
}

fn title(dialog: &Dialog) -> String {
    match (dialog.title.is_empty(), &dialog.mode) {
        (false, _) => dialog.title.clone(),
        (
            true,
            Mode::Open {
                directory: false, ..
            },
        ) => L10N.tr("title-open"),
        (true, Mode::Open { .. } | Mode::SaveMany { .. }) => L10N.tr("title-choose-folder"),
        (true, Mode::Save { .. }) => L10N.tr("title-save"),
    }
}

fn answer(selection: &Selection, dialog: &Dialog, folder: &Path) -> ExitCode {
    let remembered = match dialog.mode {
        Mode::Open {
            directory: false, ..
        }
        | Mode::Save { .. } => selection
            .paths
            .first()
            .and_then(|path| path.parent())
            .unwrap_or(folder),
        Mode::Open { .. } | Mode::SaveMany { .. } => folder,
    };
    remember_folder(&dialog.app_id, remembered);
    let mut stdout = std::io::stdout();
    match stdout
        .write_all(&encode(selection, dialog))
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::from(CHOSEN),
        Err(error) => {
            tracing::error!("cannot hand the choice to the portal: {error}");
            ExitCode::from(UNAVAILABLE)
        }
    }
}

fn run(dialog: Dialog) -> ExitCode {
    let Some(home) = std::env::home_dir() else {
        return ExitCode::from(UNAVAILABLE);
    };
    let user_dirs = read(config_home().map(|config| config.join("user-dirs.dirs")));
    let documents = user_dir(&home, user_dirs.as_deref(), UserDir::Documents);
    let remembered =
        read(memory_file()).and_then(|memory| remembered_folder(&memory, &dialog.app_id));
    let folder = initial_folder(&dialog, remembered, &home, &documents);
    let dirs = data_dirs();
    let settings = settings_folder()
        .map(|folder| FilesSettings::load(&folder))
        .unwrap_or_default();
    let chooser = Chooser::new(dialog.clone(), MimeDatabase::load(&dirs), folder, &settings);
    let bookmarks = read(config_home().map(|config| config.join("gtk-3.0/bookmarks")));
    let user = std::env::var("USER").unwrap_or_default();
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let volumes = mounted_volumes(&mountinfo, &media_roots(&user));
    let layout = std::fs::read_to_string(sidebar_file(state_home().as_deref(), &home)).ok();
    let places = with_volumes(
        &rows(
            &home,
            user_dirs.as_deref(),
            bookmarks.as_deref(),
            layout.as_deref(),
        ),
        &volumes,
        |volume| volume_icon(&mountinfo, volume),
    );
    let mut window = match Window::open(Role::toplevel(&title(&dialog), APP_ID), WINDOW_SIZE) {
        Ok(window) => window,
        Err(error) => {
            tracing::error!("{error}");
            return ExitCode::from(UNAVAILABLE);
        }
    };
    if let Some(handle) = dialog.parent_window.strip_prefix(WAYLAND_PARENT) {
        window.attach_to(handle);
    }
    let view = ChooserView::new(
        chooser,
        places,
        home.clone(),
        Icons::new(dirs),
        window.appearance(),
        TextSystem::new(),
    );
    let mut app = FileDialog { view, chosen: None };
    if let Err(error) = bande_toolkit::run(&mut window, &mut app) {
        tracing::error!("{error}");
    }
    match app.chosen {
        Some(Finished::Chosen(selection)) => {
            let folder = app.view.folder();
            answer(&selection, &dialog, &folder)
        }
        Some(Finished::Cancelled) | None => ExitCode::from(CANCELLED),
    }
}

fn confirm_wallpaper(picture: &Path, app_id: &str) -> ExitCode {
    let entries = applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    );
    let app = app_name(&entries, app_id);
    let name = picture
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let confirmed = Rc::new(Cell::new(false));
    let chosen = confirmed.clone();
    let asked = bande_toolkit::ask("wallpaper-confirmation", |window| {
        let pixels = (PREVIEW_SIZE * window.scale().max(2.0)).ceil() as u32;
        let alert = Alert::new(
            &L10N.tr_args("wallpaper-title", [("picture", name.into())]),
            &L10N.tr_args("wallpaper-message", [("app", app.into())]),
            move |action| chosen.set(action == SET),
        )
        .cancel(&L10N.tr("button-cancel"), "cancel")
        .primary(&L10N.tr("button-set-wallpaper"), SET);
        let preview = bande_wallpaper::render(picture, Fit::Fill, pixels, pixels)
            .ok()
            .and_then(|picture| Image::from_rgba(picture.width, picture.height, picture.rgba));
        match preview {
            Some(preview) => alert.icon(preview),
            None => alert,
        }
    });
    match asked {
        Ok(()) if confirmed.get() => ExitCode::from(CHOSEN),
        Ok(()) => ExitCode::from(CANCELLED),
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::from(UNAVAILABLE)
        }
    }
}

fn main() -> ExitCode {
    bande_log::init("bande-file-chooser");
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    if let [flag, picture, app_flag, app_id] = arguments.as_slice()
        && flag == SET_WALLPAPER
        && app_flag == "--app-id"
    {
        return confirm_wallpaper(Path::new(picture), &app_id.to_string_lossy());
    }
    let mode = arguments.first().cloned().unwrap_or_default();
    if mode == ACCESS {
        return match parse_access(arguments) {
            Ok(question) => access::ask(&question),
            Err(error) => {
                tracing::error!("{}", error.0);
                ExitCode::from(UNAVAILABLE)
            }
        };
    }
    if mode == PRINT {
        return print::run(arguments);
    }
    if mode == WORKER_FLAG {
        return match arguments.get(1) {
            Some(path) => bande_content::pdf::worker::run(Path::new(path)),
            None => ExitCode::from(UNAVAILABLE),
        };
    }
    if mode == CHOOSE_APP {
        return match parse_app_chooser(arguments) {
            Ok(offer) => app_chooser::choose(&offer),
            Err(error) => {
                tracing::error!("{}", error.0);
                ExitCode::from(UNAVAILABLE)
            }
        };
    }
    if mode == SHARE_ACCOUNT {
        return match parse_account(arguments) {
            Ok(details) => account::share(&details),
            Err(error) => {
                tracing::error!("{}", error.0);
                ExitCode::from(UNAVAILABLE)
            }
        };
    }
    if mode == CREATE_LAUNCHER {
        return match parse_launcher(arguments) {
            Ok(launcher) => launcher::confirm(&launcher),
            Err(error) => {
                tracing::error!("{}", error.0);
                ExitCode::from(UNAVAILABLE)
            }
        };
    }
    match parse(arguments) {
        Ok(dialog) => run(dialog),
        Err(error) => {
            tracing::error!("{}", error.0);
            ExitCode::from(UNAVAILABLE)
        }
    }
}
