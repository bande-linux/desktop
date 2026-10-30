use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use accesskit::TreeUpdate;
use unbound_file_chooser::{
    Chooser, ChooserView, Finished, Icons, L10N, initial_folder, remember, remembered_folder,
};
use unbound_files::{rows, with_volumes};
use unbound_toolkit::{App, Role, Scene, TextSystem, Window, WindowEvent};
use unbound_volumes::{media_roots, mounted_volumes};
use unbound_xdg::{MimeDatabase, UserDir, config_home, data_dirs, user_dir};
use xdg_desktop_portal_unbound::dialog_process::{CANCELLED, CHOSEN, encode, parse};
use xdg_desktop_portal_unbound::file_chooser::{Dialog, Mode, Selection};

const APP_ID: &str = "org.unbound.FileChooser";
const WINDOW_SIZE: (u32, u32) = (820, 520);
const UNAVAILABLE: u8 = 2;

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
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|state| state.is_absolute())
        .or_else(|| std::env::home_dir().map(|home| home.join(".local/state")))
        .map(|state| state.join("unbound/file-chooser-folders"))
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
    let places = with_volumes(
        &rows(&home, user_dirs.as_deref(), bookmarks.as_deref()),
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

fn main() -> ExitCode {
    unbound_log::init("unbound-file-chooser");
    match parse(std::env::args_os().skip(1)) {
        Ok(dialog) => run(dialog),
        Err(error) => {
            tracing::error!("{}", error.0);
            ExitCode::from(UNAVAILABLE)
        }
    }
}
