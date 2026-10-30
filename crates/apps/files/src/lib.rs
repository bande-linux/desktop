mod browser;
mod context;
mod controls;
mod drag;
mod grid;
mod icons;
mod loader;
mod menus;
mod paint;
mod prompt;
mod sidebar;
mod tabs;
mod thumbs;
mod view;
mod views;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use browser::{Browser, Column, Location};
pub use paint::permissions;
pub use sidebar::{Place, PlaceKind, Row, rows, with_volumes, without_bookmark};
pub use tabs::TAB_BAR_HEIGHT;
pub use view::{
    COLUMN_WIDTH, FilesView, Hit, Info, PATH_BAR_HEIGHT, QuickLook, ROW_HEIGHT, Rename, Request,
    SIDEBAR_WIDTH, Sheet,
};
pub use views::{FolderView, ViewMemory, ViewMode, views_file};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-files.ftl");

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
            include_str!("grid.rs"),
            include_str!("tabs.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
