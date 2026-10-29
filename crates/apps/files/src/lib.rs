mod apps;
mod browser;
mod editor;
mod icons;
mod listing;
mod loader;
mod operations;
mod paint;
mod preview;
mod sidebar;
mod view;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use apps::Apps;
pub use browser::{Browser, Column, Location};
pub use editor::NameEditor;
pub use listing::{
    Entry, Kind, Sort, SortBy, finder_order, format_size, is_hidden, list, list_trash,
};
pub use operations::{Operations, Purpose};
pub use paint::{format_date, permissions};
pub use preview::{Preview, TEXT_PREVIEW_BYTES, preview};
pub use sidebar::{Place, Row, rows};
pub use view::{
    COLUMN_WIDTH, FilesView, Hit, Info, QuickLook, ROW_HEIGHT, Rename, Request, SIDEBAR_WIDTH,
    Sheet,
};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-files.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("lib.rs"),
            include_str!("main.rs"),
            include_str!("browser.rs"),
            include_str!("listing.rs"),
            include_str!("paint.rs"),
            include_str!("sidebar.rs"),
            include_str!("view.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
