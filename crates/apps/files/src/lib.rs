mod browser;
mod context;
mod controls;
mod drag;
mod grid;
mod icons;
mod lister;
mod loader;
mod menus;
mod paint;
mod prompt;
mod search;
mod sidebar;
mod tabs;
mod thumbs;
mod toolbar;
mod view;
mod views;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use browser::{Browser, Column, Location, RowEdit};
pub use paint::permissions;
pub use search::SEARCH_HEIGHT;
pub use sidebar::{
    BookmarkEdit, Place, PlaceKind, Row, SidebarEdit, favourites, hidden_places, layout_text, rows,
    with_bookmarks, with_volumes, without_bookmark,
};
pub use tabs::TAB_BAR_HEIGHT;
pub use toolbar::TOOLBAR_HEIGHT;
pub use view::{
    COLUMN_WIDTH, ColumnPreview, FilesView, Hit, Info, PATH_BAR_HEIGHT, PREVIEW_WIDTH, QuickLook,
    ROW_HEIGHT, Rename, Request, SIDEBAR_WIDTH, Sheet,
};
pub use views::{
    FolderView, ListColumn, ViewMemory, ViewMode, list_columns_file, sidebar_file, status_bar_file,
    views_file,
};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-files.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
