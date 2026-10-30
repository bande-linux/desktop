mod about;
mod arrangement;
mod battery;
mod client;
mod clock;
mod displays;
mod follow;
mod general;
mod idle;
mod login;
mod pages;
mod pictures;
mod pointing;
mod power;
mod services;
mod state;
mod storage;
mod time;
mod view;
mod widgets;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use about::{About, AboutCommand, chip_name, memory_bytes, static_hostname, system_version};
pub use battery::{BatteryCommand, BatteryState, Charge, charge_of, follow_battery};
pub use client::{CONFIG_SETTINGS, SettingsClient};
pub use displays::Displays;
pub use follow::{Command, Feed, Services, Update, Updates, feed, follow_config, worker};
pub use idle::Idle;
pub use login::{App, LoginCommand, LoginItems, login_items, login_matches};
pub use pictures::{Pair, PictureCommand, bundled_pairs, thumbnail};
pub use pointing::{PointingDevices, UDEV_DATA};
pub use power::charge_text;
pub use services::{
    AudioCommand, BluetoothCommand, NetworkCommand, follow_bluetooth, follow_network,
};
pub use state::{Answer, BluetoothState, Display, Joined, Mode, Question, WifiState};
pub use storage::{Category, Storage, StorageCommand, Tally, categorize, tally};
pub use time::{
    TimeCommand, TimeState, follow_time, parse_manual, zone_matches, zone_offset, zone_place,
};
pub use unbound_settings_panes::{PaneId, SettingId, Value};
pub use view::SettingsView;
pub use widgets::bucket_levels;

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
            include_str!("clock.rs"),
            include_str!("follow.rs"),
            include_str!("general.rs"),
            include_str!("pages.rs"),
            include_str!("pictures.rs"),
            include_str!("power.rs"),
            include_str!("view.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
