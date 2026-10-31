use std::cell::Cell;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Instant;

use accesskit::TreeUpdate;
use unbound_file_chooser::{
    Chooser, ChooserView, Finished, Icons, L10N, initial_folder, remember, remembered_folder,
};
use unbound_files::{rows, sidebar_file, with_volumes};
use unbound_toolkit::ui::Alert;
use unbound_toolkit::{App, Image, Role, Scene, TextSystem, Window, WindowEvent};
use unbound_volumes::{media_roots, mounted_volumes};
use unbound_wallpaper::Fit;
use unbound_xdg::{
    Locale, MimeDatabase, UserDir, app_name, applications, config_home, current_desktops,
    data_dirs, state_home, user_dir,
};
use xdg_desktop_portal_unbound::dialog_process::{CANCELLED, CHOSEN, SET_WALLPAPER, encode, parse};
use xdg_desktop_portal_unbound::file_chooser::{Dialog, Mode, Selection};

const APP_ID: &str = "org.unbound.FileChooser";
const WINDOW_SIZE: (u32, u32) = (820, 520);
const UNAVAILABLE: u8 = 2;
const PREVIEW_SIZE: f64 = 36.0;
const SET: &str = "set";

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

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.view.accessibility())
    }
}

fn memory_file() -> Option<PathBuf> {
    unbound_xdg::state_home().map(|state| state.join("unbound/file-chooser-folders"))
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
    let staging = file.with_extension("new");
    let written = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&staging, updated))
        .and_then(|()| std::fs::rename(&staging, &file));
    if let Err(error) = written {
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
    let chooser = Chooser::new(dialog.clone(), MimeDatabase::load(&dirs), folder);
    let bookmarks = read(config_home().map(|config| config.join("gtk-3.0/bookmarks")));
    let user = std::env::var("USER").unwrap_or_default();
    let volumes = mounted_volumes(
        &std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default(),
        &media_roots(&user),
    );
    let layout = std::fs::read_to_string(sidebar_file(state_home().as_deref(), &home)).ok();
    let places = with_volumes(
        &rows(
            &home,
            user_dirs.as_deref(),
            bookmarks.as_deref(),
            layout.as_deref(),
        ),
        &volumes,
    );
    let mut window = match Window::open(Role::toplevel(&title(&dialog), APP_ID), WINDOW_SIZE) {
        Ok(window) => window,
        Err(error) => {
            tracing::error!("{error}");
            return ExitCode::from(UNAVAILABLE);
        }
    };
    let view = ChooserView::new(
        chooser,
        places,
        home.clone(),
        Icons::new(dirs),
        window.appearance(),
        TextSystem::new(),
    );
    let mut app = FileDialog { view, chosen: None };
    if let Err(error) = unbound_toolkit::run(&mut window, &mut app) {
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
    let asked = unbound_toolkit::ask("wallpaper-confirmation", |window| {
        let pixels = (PREVIEW_SIZE * window.scale().max(2.0)).ceil() as u32;
        let alert = Alert::new(
            &L10N.tr_args("wallpaper-title", [("picture", name.into())]),
            &L10N.tr_args("wallpaper-message", [("app", app.into())]),
            move |action| chosen.set(action == SET),
        )
        .cancel(&L10N.tr("button-cancel"), "cancel")
        .primary(&L10N.tr("button-set-wallpaper"), SET);
        let preview = unbound_wallpaper::render(picture, Fit::Fill, pixels, pixels)
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
    unbound_log::init("unbound-file-chooser");
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    if let [flag, picture, app_flag, app_id] = arguments.as_slice()
        && flag == SET_WALLPAPER
        && app_flag == "--app-id"
    {
        return confirm_wallpaper(Path::new(picture), &app_id.to_string_lossy());
    }
    match parse(arguments) {
        Ok(dialog) => run(dialog),
        Err(error) => {
            tracing::error!("{}", error.0);
            ExitCode::from(UNAVAILABLE)
        }
    }
}
