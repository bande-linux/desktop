mod client;
mod idle;
mod registry;
mod search;
mod view;
mod widgets;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use client::{CONFIG_SETTINGS, SettingsClient};
pub use idle::Idle;
pub use registry::{Choice, Kind, Pane, PaneId, Section, Setting, SettingId, Value, panes};
pub use search::{PaneHit, SettingHit, search};
pub use view::SettingsView;

const ENGLISH: &str = include_str!("../locales/en-US/unbound-settings.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("lib.rs"),
            include_str!("main.rs"),
            include_str!("client.rs"),
            include_str!("registry.rs"),
            include_str!("view.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
