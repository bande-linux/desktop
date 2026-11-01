use std::path::Path;

use bande_app_settings::{Shared, Stored, flag, pairs};
use bande_toolkit::ui::{
    Context, Form, FormLine, PopUpButton, ScrollView, SegmentedControl, Toggle, Ui, Widget,
};
use bande_toolkit::{Appearance, TextSystem};
use bande_xdg::{UserDir, user_dir};

use crate::L10N;
use crate::browser::Location;
use crate::views::ViewMode;

pub const SETTINGS_SIZE: (u32, u32) = (500, 460);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartFolder {
    #[default]
    Home,
    Desktop,
    Documents,
    Downloads,
    Recents,
}

impl StartFolder {
    pub const ALL: [Self; 5] = [
        Self::Home,
        Self::Desktop,
        Self::Documents,
        Self::Downloads,
        Self::Recents,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Desktop => "desktop",
            Self::Documents => "documents",
            Self::Downloads => "downloads",
            Self::Recents => "recents",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Home => "settings-home",
            Self::Desktop => "menu-desktop",
            Self::Documents => "menu-documents",
            Self::Downloads => "menu-downloads",
            Self::Recents => "recents",
        }
    }

    pub fn location(self, home: &Path, user_dirs: Option<&str>) -> Location {
        let folder = |dir| Location::Folder(user_dir(home, user_dirs, dir));
        match self {
            Self::Home => Location::Folder(home.to_path_buf()),
            Self::Desktop => folder(UserDir::Desktop),
            Self::Documents => folder(UserDir::Documents),
            Self::Downloads => folder(UserDir::Downloads),
            Self::Recents => Location::Recents,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilesSettings {
    pub new_window: StartFolder,
    pub default_view: ViewMode,
    pub show_hidden: bool,
    pub warn_empty_trash: bool,
    pub purge_trash: bool,
    pub search_home: bool,
}

impl Default for FilesSettings {
    fn default() -> Self {
        Self {
            new_window: StartFolder::default(),
            default_view: ViewMode::default(),
            show_hidden: false,
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
                        .find(|folder| folder.name() == value)
                        .unwrap_or(settings.new_window);
                }
                "default-view" => {
                    settings.default_view = ViewMode::parse(value).unwrap_or(settings.default_view);
                }
                "show-hidden" => settings.show_hidden = flag(value).unwrap_or(settings.show_hidden),
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
        format!(
            "new-window = {}\ndefault-view = {}\nshow-hidden = {}\nwarn-empty-trash = {}\npurge-trash = {}\nsearch-in = {}\n",
            self.new_window.name(),
            self.default_view.name(),
            self.show_hidden,
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

fn form(settings: &Shared<FilesSettings>) -> Form {
    let current = *settings.borrow();
    let folders: Vec<String> = StartFolder::ALL
        .iter()
        .map(|folder| L10N.tr(folder.message()))
        .collect();
    let shared = settings.clone();
    let new_window = choice(
        "settings-new-window",
        &folders,
        StartFolder::ALL
            .iter()
            .position(|folder| *folder == current.new_window)
            .unwrap_or_default(),
        |label, choices, selected| {
            Box::new(PopUpButton::new(label, choices, selected, move |index| {
                shared.borrow_mut().new_window =
                    StartFolder::ALL.get(index).copied().unwrap_or_default();
            }))
        },
    );
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
    let heading = |message: &str| FormLine::Heading(L10N.tr(message));
    Form::new(
        &L10N.tr("settings-title"),
        vec![
            heading("settings-general"),
            new_window,
            default_view,
            toggle(
                settings,
                "settings-show-hidden",
                current.show_hidden,
                |settings, on| settings.show_hidden = on,
            ),
            heading("settings-trash"),
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
            heading("settings-search"),
            search_in,
        ],
    )
}

pub fn settings_view(appearance: Appearance, settings: &Shared<FilesSettings>) -> Ui {
    let mut ui = Ui::new(
        Box::new(ScrollView::new(form(settings))),
        Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    ui
}
