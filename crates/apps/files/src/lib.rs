mod browser;
mod listing;
mod preview;
mod view;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use browser::{Browser, Column};
pub use listing::{Entry, Kind, finder_order, format_size, is_hidden, list};
pub use preview::{Preview, TEXT_PREVIEW_BYTES, preview};
pub use view::{COLUMN_WIDTH, FilesView, QuickLook, ROW_HEIGHT};

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
            include_str!("preview.rs"),
            include_str!("view.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
