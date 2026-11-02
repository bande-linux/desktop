use std::fs;
use std::path::{Path, PathBuf};
use std::thread;

use bande_app::choose_file;
use bande_app_settings::{Shared, Stored, flag, pairs, settings_folder};
use bande_toolkit::ui::{
    Context, Form, FormLine, PopUpButton, ScrollView, SegmentedControl, Slider, Toggle, Ui, Widget,
};
use bande_toolkit::{Appearance, TextSystem};
use bande_xdg::{UserDir, config_home, replace_file, state_home, user_dir};

use crate::L10N;
use crate::browser::Location;
use crate::sidebar::{hidden_places, standard_places, volumes_hidden, with_place_shown};
use crate::view::home;
use crate::views::{ViewMode, sidebar_file};

pub const SETTINGS_SIZE: (u32, u32) = (500, 560);
pub const DEFAULT_SPRING_DELAY: u32 = 900;
const SPRING_DELAYS: (u32, u32) = (200, 1600);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartFolder {
    #[default]
    Home,
    Desktop,
    Documents,
    Downloads,
    Recents,
    Last,
    Other,
}

impl StartFolder {
    pub const ALL: [Self; 6] = [
        Self::Home,
        Self::Desktop,
        Self::Documents,
        Self::Downloads,
        Self::Recents,
        Self::Last,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Desktop => "desktop",
            Self::Documents => "documents",
            Self::Downloads => "downloads",
            Self::Recents => "recents",
            Self::Last => "last",
            Self::Other => "other",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Home => "settings-home",
            Self::Desktop => "menu-desktop",
            Self::Documents => "menu-documents",
            Self::Downloads => "menu-downloads",
            Self::Recents => "recents",
            Self::Last => "settings-last-folder",
            Self::Other => "settings-other-folder",
        }
    }

    pub fn location(
        self,
        home: &Path,
        user_dirs: Option<&str>,
        last: Option<&Path>,
        other: Option<&Path>,
    ) -> Location {
        let folder = |dir| Location::Folder(user_dir(home, user_dirs, dir));
        let existing = |folder: Option<&Path>| {
            Location::Folder(
                folder
                    .filter(|folder| folder.is_dir())
                    .unwrap_or(home)
                    .to_path_buf(),
            )
        };
        match self {
            Self::Home => Location::Folder(home.to_path_buf()),
            Self::Desktop => folder(UserDir::Desktop),
            Self::Documents => folder(UserDir::Documents),
            Self::Downloads => folder(UserDir::Downloads),
            Self::Recents => Location::Recents,
            Self::Last => existing(last),
            Self::Other => existing(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesSettings {
    pub new_window: StartFolder,
    pub new_window_folder: Option<PathBuf>,
    pub default_view: ViewMode,
    pub show_hidden: bool,
    pub show_extensions: bool,
    pub desktop_disks: bool,
    pub spring_loading: bool,
    pub spring_delay: u32,
    pub warn_extension_change: bool,
    pub folders_first: bool,
    pub warn_empty_trash: bool,
    pub purge_trash: bool,
    pub search_home: bool,
}

impl Default for FilesSettings {
    fn default() -> Self {
        Self {
            new_window: StartFolder::default(),
            new_window_folder: None,
            default_view: ViewMode::default(),
            show_hidden: false,
            show_extensions: false,
            desktop_disks: true,
            spring_loading: true,
            spring_delay: DEFAULT_SPRING_DELAY,
            warn_extension_change: true,
            folders_first: true,
            warn_empty_trash: true,
            purge_trash: false,
            search_home: false,
        }
    }
}

impl Stored for FilesSettings {
    const FILE: &'static str = "files.conf";

    fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for (key, value) in pairs(text) {
            match key {
                "new-window" => {
                    settings.new_window = StartFolder::ALL
                        .into_iter()
                        .chain([StartFolder::Other])
                        .find(|folder| folder.name() == value)
                        .unwrap_or(settings.new_window);
                }
                "new-window-folder" => {
                    settings.new_window_folder =
                        Some(PathBuf::from(value)).filter(|folder| folder.is_absolute());
                }
                "default-view" => {
                    settings.default_view = ViewMode::parse(value).unwrap_or(settings.default_view);
                }
                "show-hidden" => settings.show_hidden = flag(value).unwrap_or(settings.show_hidden),
                "show-extensions" => {
                    settings.show_extensions = flag(value).unwrap_or(settings.show_extensions);
                }
                "spring-loading" => {
                    settings.spring_loading = flag(value).unwrap_or(settings.spring_loading);
                }
                "spring-delay" => {
                    settings.spring_delay =
                        value.parse::<u32>().map_or(settings.spring_delay, |delay| {
                            delay.clamp(SPRING_DELAYS.0, SPRING_DELAYS.1)
                        });
                }
                "desktop-disks" => {
                    settings.desktop_disks = flag(value).unwrap_or(settings.desktop_disks);
                }
                "warn-extension-change" => {
                    settings.warn_extension_change =
                        flag(value).unwrap_or(settings.warn_extension_change);
                }
                "folders-first" => {
                    settings.folders_first = flag(value).unwrap_or(settings.folders_first);
                }
                "warn-empty-trash" => {
                    settings.warn_empty_trash = flag(value).unwrap_or(settings.warn_empty_trash);
                }
                "purge-trash" => settings.purge_trash = flag(value).unwrap_or(settings.purge_trash),
                "search-in" => {
                    settings.search_home = match value {
                        "home" => true,
                        "folder" => false,
                        _ => settings.search_home,
                    };
                }
                _ => {}
            }
        }
        settings
    }

    fn to_text(&self) -> String {
        let folder = self
            .new_window_folder
            .as_ref()
            .map(|folder| format!("new-window-folder = {}\n", folder.display()))
            .unwrap_or_default();
        folder
            + &format!(
                "new-window = {}\ndefault-view = {}\nshow-hidden = {}\nshow-extensions = {}\ndesktop-disks = {}\nspring-loading = {}\nspring-delay = {}\nwarn-extension-change = {}\nfolders-first = {}\nwarn-empty-trash = {}\npurge-trash = {}\nsearch-in = {}\n",
                self.new_window.name(),
                self.default_view.name(),
                self.show_hidden,
                self.show_extensions,
                self.desktop_disks,
                self.spring_loading,
                self.spring_delay,
                self.warn_extension_change,
                self.folders_first,
                self.warn_empty_trash,
                self.purge_trash,
                if self.search_home { "home" } else { "folder" },
            )
    }
}

fn toggle(
    settings: &Shared<FilesSettings>,
    message: &str,
    on: bool,
    set: fn(&mut FilesSettings, bool),
) -> FormLine {
    let shared = settings.clone();
    let label = L10N.tr(message);
    let control = Toggle::new(&label, on, move |on| set(&mut shared.borrow_mut(), on));
    FormLine::Row(label, Box::new(control))
}

fn choice(
    message: &str,
    choices: &[String],
    selected: usize,
    control: impl FnOnce(&str, &[String], usize) -> Box<dyn Widget>,
) -> FormLine {
    let label = L10N.tr(message);
    let widget = control(&label, choices, selected);
    FormLine::Row(label, widget)
}

fn place_toggle(file: &Path, label: String, place: Option<Location>, shown: bool) -> FormLine {
    let file = file.to_path_buf();
    let control = Toggle::new(&label, shown, move |on| {
        let layout = fs::read_to_string(&file).unwrap_or_default();
        if let Err(error) = replace_file(&file, with_place_shown(&layout, place.as_ref(), on)) {
            tracing::warn!("cannot change the sidebar: {error}");
        }
    });
    FormLine::Row(label, Box::new(control))
}

fn sidebar_lines() -> Vec<FormLine> {
    let home = home();
    let file = sidebar_file(state_home().as_deref(), &home);
    let layout = fs::read_to_string(&file).ok();
    let hidden = hidden_places(layout.as_deref());
    let user_dirs =
        config_home().and_then(|config| fs::read_to_string(config.join("user-dirs.dirs")).ok());
    let mut lines = vec![FormLine::Heading(L10N.tr("settings-sidebar"))];
    for place in standard_places(&home, user_dirs.as_deref()) {
        let shown = !hidden.contains(&place.location);
        lines.push(place_toggle(&file, place.name, Some(place.location), shown));
    }
    lines.push(place_toggle(
        &file,
        L10N.tr("settings-external-disks"),
        None,
        !volumes_hidden(layout.as_deref()),
    ));
    lines
}

fn choose_new_window_folder(previous: FilesSettings) {
    let Some(folder) = settings_folder() else {
        return;
    };
    let title = L10N.tr("settings-choose-new-window-folder");
    let spawned = thread::Builder::new()
        .name("new window folder".to_owned())
        .spawn(move || {
            let chosen = zbus::blocking::Connection::session()
                .and_then(|session| choose_file(&session, &title, None))
                .inspect_err(|error| tracing::warn!("cannot choose a folder: {error}"));
            let mut stored = FilesSettings::load(&folder);
            match chosen {
                Ok(Some(chosen)) => stored.new_window_folder = Some(chosen),
                _ => {
                    stored.new_window = previous.new_window;
                    stored.new_window_folder = previous.new_window_folder;
                }
            }
            if let Err(error) = stored.save(&folder) {
                tracing::warn!("cannot save the Files settings: {error}");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("cannot choose a folder: {error}");
    }
}

fn new_window_line(settings: &Shared<FilesSettings>) -> FormLine {
    let current = settings.borrow().clone();
    let standard = StartFolder::ALL.len();
    let mut folders: Vec<String> = StartFolder::ALL
        .iter()
        .map(|folder| L10N.tr(folder.message()))
        .collect();
    let chosen = current.new_window_folder.as_ref().map(|folder| {
        folder.file_name().map_or_else(
            || folder.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    });
    let other = standard + usize::from(chosen.is_some());
    folders.extend(chosen);
    folders.push(L10N.tr(StartFolder::Other.message()));
    let selected = match current.new_window {
        StartFolder::Other => standard,
        shown => StartFolder::ALL
            .iter()
            .position(|folder| *folder == shown)
            .unwrap_or_default(),
    };
    let shared = settings.clone();
    choice(
        "settings-new-window",
        &folders,
        selected,
        |label, choices, selected| {
            Box::new(PopUpButton::new(label, choices, selected, move |index| {
                let previous = shared.borrow().clone();
                let mut settings = shared.borrow_mut();
                settings.new_window = match StartFolder::ALL.get(index) {
                    Some(folder) => *folder,
                    None => StartFolder::Other,
                };
                if index == other {
                    settings.new_window_folder = None;
                    choose_new_window_folder(previous);
                }
            }))
        },
    )
}

fn form(settings: &Shared<FilesSettings>) -> Form {
    let current = settings.borrow().clone();
    let new_window = new_window_line(settings);
    let views: Vec<String> = ViewMode::ALL
        .iter()
        .map(|mode| L10N.tr(mode.message()))
        .collect();
    let shared = settings.clone();
    let default_view = choice(
        "settings-default-view",
        &views,
        ViewMode::ALL
            .iter()
            .position(|mode| *mode == current.default_view)
            .unwrap_or_default(),
        |label, choices, selected| {
            Box::new(SegmentedControl::new(
                label,
                choices,
                Some(selected),
                move |index| {
                    shared.borrow_mut().default_view =
                        ViewMode::ALL.get(index).copied().unwrap_or_default();
                },
            ))
        },
    );
    let scopes = [L10N.tr("settings-current-folder"), L10N.tr("settings-home")];
    let shared = settings.clone();
    let search_in = choice(
        "settings-search-in",
        &scopes,
        usize::from(current.search_home),
        |label, choices, selected| {
            Box::new(SegmentedControl::new(
                label,
                choices,
                Some(selected),
                move |index| shared.borrow_mut().search_home = index == 1,
            ))
        },
    );
    let shared = settings.clone();
    let label = L10N.tr("settings-spring-delay");
    let spring_delay = FormLine::Row(
        label.clone(),
        Box::new(Slider::new(
            &label,
            f64::from(current.spring_delay) / 1000.0,
            (
                f64::from(SPRING_DELAYS.0) / 1000.0,
                f64::from(SPRING_DELAYS.1) / 1000.0,
            ),
            0.1,
            move |seconds| shared.borrow_mut().spring_delay = (seconds * 1000.0).round() as u32,
        )),
    );
    let heading = |message: &str| FormLine::Heading(L10N.tr(message));
    let mut lines = vec![
        heading("settings-general"),
        new_window,
        default_view,
        toggle(
            settings,
            "settings-show-hidden",
            current.show_hidden,
            |settings, on| settings.show_hidden = on,
        ),
        toggle(
            settings,
            "settings-desktop-disks",
            current.desktop_disks,
            |settings, on| settings.desktop_disks = on,
        ),
        toggle(
            settings,
            "settings-spring-loading",
            current.spring_loading,
            |settings, on| settings.spring_loading = on,
        ),
        spring_delay,
    ];
    lines.extend(sidebar_lines());
    lines.extend([
        heading("settings-advanced"),
        toggle(
            settings,
            "settings-show-extensions",
            current.show_extensions,
            |settings, on| settings.show_extensions = on,
        ),
        toggle(
            settings,
            "settings-warn-extension-change",
            current.warn_extension_change,
            |settings, on| settings.warn_extension_change = on,
        ),
        toggle(
            settings,
            "settings-warn-empty-trash",
            current.warn_empty_trash,
            |settings, on| settings.warn_empty_trash = on,
        ),
        toggle(
            settings,
            "settings-purge-trash",
            current.purge_trash,
            |settings, on| settings.purge_trash = on,
        ),
        toggle(
            settings,
            "settings-folders-first",
            current.folders_first,
            |settings, on| settings.folders_first = on,
        ),
        search_in,
    ]);
    Form::new(&L10N.tr("settings-title"), lines)
}

pub fn settings_view(appearance: Appearance, settings: &Shared<FilesSettings>) -> Ui {
    let mut ui = Ui::new(
        Box::new(ScrollView::new(form(settings))),
        Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    ui
}
