mod browser;
mod context;
mod icons;
mod loader;
mod menus;
mod paint;
mod sidebar;
mod view;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use browser::{Browser, Column, Location};
pub use paint::permissions;
pub use sidebar::{Place, Row, rows, without_bookmark};
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
            include_str!("menus.rs"),
            include_str!("browser.rs"),
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
