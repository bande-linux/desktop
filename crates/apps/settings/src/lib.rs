mod client;
mod model;
mod view;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use client::SettingsClient;
pub use model::{Choice, Control, Key, Match, Pane, Value, panes, search};
pub use view::{SIDEBAR_WIDTH, SettingsView};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-settings.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("lib.rs"),
            include_str!("main.rs"),
            include_str!("model.rs"),
            include_str!("view.rs"),
            include_str!("client.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
