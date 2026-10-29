mod model;
mod view;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use model::{Action, AppInfo, Heading, Hit, Kind, Target};
pub use view::{CARD_WIDTH, Catalog, ICON_SIZE, Launcher, Loaded};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-launcher.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("lib.rs"),
            include_str!("main.rs"),
            include_str!("view.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
