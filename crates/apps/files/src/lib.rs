mod batch_rename;
mod browser;
mod context;
mod controls;
mod drag;
mod grid;
mod icons;
mod inspector;
mod lister;
mod loader;
mod menus;
mod open_with;
mod paint;
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
pub use open_with::OpenWith;
pub use paint::permissions;
pub use quick_look::QuickLook;
pub use search::SEARCH_HEIGHT;
pub use settings::{FilesSettings, SETTINGS_SIZE, StartFolder, settings_view};
pub use sidebar::{
    BookmarkEdit, Place, PlaceKind, Row, SidebarEdit, favourites, hidden_places, layout_text, rows,
    with_bookmarks, with_volumes, without_bookmark,
};
pub use tabs::TAB_BAR_HEIGHT;
pub use toolbar::TOOLBAR_HEIGHT;
pub use view::{
    COLUMN_WIDTH, ColumnPreview, FilesView, Hit, Info, PATH_BAR_HEIGHT, PREVIEW_WIDTH, ROW_HEIGHT,
    Rename, Request, SIDEBAR_WIDTH, Sheet,
};
pub use views::{
    FolderView, ListColumn, ViewMemory, ViewMode, list_columns_file, sidebar_file, status_bar_file,
    views_file,
};

const ENGLISH: &str = include_str!("../locales/en-US/bande-files.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("lib.rs"),
            include_str!("main.rs"),
            include_str!("menu_bar.rs"),
            include_str!("menus.rs"),
            include_str!("browser.rs"),
            include_str!("controls.rs"),
            include_str!("paint.rs"),
            include_str!("sidebar.rs"),
            include_str!("view.rs"),
            include_str!("settings.rs"),
            include_str!("grid.rs"),
            include_str!("inspector.rs"),
            include_str!("batch_rename.rs"),
            include_str!("open_with.rs"),
            include_str!("quick_look.rs"),
            include_str!("tabs.rs"),
            include_str!("search.rs"),
            include_str!("toolbar.rs"),
        ];
        assert_eq!(
            bande_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
