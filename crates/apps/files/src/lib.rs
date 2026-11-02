mod batch_rename;
mod browser;
mod context;
mod controls;
mod drag;
mod file_manager;
mod grid;
mod icons;
mod inspector;
mod lister;
mod loader;
mod menus;
mod open_with;
mod paint;
mod privacy;
mod prompt;
mod quick_look;
mod search;
mod settings;
mod sidebar;
mod tabs;
mod thumbs;
mod toolbar;
mod view;
mod views;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use batch_rename::{BatchRename, RuleKind};
pub use browser::{Browser, Column, Location, RowEdit};
pub use file_manager::{Show, ShowRequest, serve_file_manager};
pub use open_with::OpenWith;
pub use paint::permissions;
pub use privacy::follow_search_excluded;
pub use quick_look::QuickLook;
pub use search::SEARCH_HEIGHT;
pub use settings::{FilesSettings, SETTINGS_SIZE, StartFolder, settings_view};
pub use sidebar::{
    BookmarkEdit, Place, PlaceKind, Row, SidebarEdit, favourites, hidden_places, layout_text, rows,
    standard_places, volumes_hidden, with_bookmarks, with_place_shown, with_volumes,
    without_bookmark,
};
pub use tabs::TAB_BAR_HEIGHT;
pub use toolbar::TOOLBAR_HEIGHT;
pub use view::{
    COLUMN_WIDTH, ColumnPreview, FilesView, Hit, Info, PATH_BAR_HEIGHT, PREVIEW_WIDTH, ROW_HEIGHT,
    Rename, Request, SIDEBAR_WIDTH, Sheet,
};
pub use views::{
    FolderView, ListColumn, SIDEBAR_FILE, ViewMemory, ViewMode, last_folder_file,
    list_columns_file, sidebar_file, status_bar_file, views_file,
};

const ENGLISH: &str = include_str!("../locales/en-US/bande-files.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
