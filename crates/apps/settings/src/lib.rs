mod client;
mod displays;
mod follow;
mod idle;
mod pages;
mod registry;
mod search;
mod services;
mod state;
mod view;
mod widgets;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use client::{CONFIG_SETTINGS, SettingsClient};
pub use displays::Displays;
pub use follow::{Command, Feed, Services, Update, Updates, feed, follow_config, worker};
pub use idle::Idle;
pub use registry::{Choice, Kind, List, Pane, PaneId, Section, Setting, SettingId, Value, panes};
pub use search::{PaneHit, SettingHit, search};
pub use services::{
    AudioCommand, BluetoothCommand, NetworkCommand, follow_bluetooth, follow_network,
};
pub use state::{Answer, BluetoothState, Display, Joined, Mode, Question, WifiState};
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
            include_str!("follow.rs"),
            include_str!("pages.rs"),
            include_str!("registry.rs"),
            include_str!("view.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
