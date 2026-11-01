mod about;
mod arrangement;
mod battery;
mod brightness;
mod client;
mod clock;
mod defaults;
mod displays;
mod failure;
mod follow;
mod forget;
mod general;
mod header;
mod idle;
mod language;
mod locale;
mod login;
mod network;
mod night;
mod notifications;
mod pages;
mod permissions;
mod pictures;
mod pointing;
mod power;
mod privacy;
mod privilege;
mod services;
mod software;
mod state;
mod storage;
mod time;
mod updates;
mod view;
mod widgets;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use about::{About, AboutCommand, chip_name, memory_bytes, static_hostname, system_version};
pub use bande_settings_panes::{PaneId, SettingId, Value};
pub use battery::{BatteryCommand, BatteryState, Charge, charge_of, follow_battery};
pub use client::{CONFIG_SETTINGS, SettingsClient};
pub use defaults::{DefaultApp, DefaultAppCommand, choose_default, default_apps};
pub use displays::Displays;
pub use follow::{Command, Feed, Services, Update, Updates, feed, follow_config, worker};
pub use idle::Idle;
pub use locale::{
    AvailableLocale, LocaleCommand, LocaleState, assignments, locale_name, login_codes,
    parse_assignments, supported_locales, x11_keyboard,
};
pub use login::{App, LoginCommand, LoginItems, login_items, login_matches};
pub use night::{clock_text, night_summary};
pub use notifications::{AppNotifications, NotificationCommand, follow_app_notifications};
pub use permissions::{Grant, Grants, PermissionCommand, follow_permissions};
pub use pictures::{Pair, PictureCommand, bundled_pairs, thumbnail};
pub use pointing::{PointingDevices, UDEV_DATA, follow_pointing};
pub use power::charge_text;
pub use privilege::{Privilege, refused};
pub use services::{
    AudioCommand, BluetoothCommand, NetworkCommand, follow_bluetooth, follow_network,
};
pub use software::{
    Need, Package, Reboot, Refresh, SoftwareCommand, SoftwareState, UpdateNotice, announce_updates,
    check, follow_software, open_updates, update,
};
pub use state::{Answer, BluetoothState, Display, Joined, Mode, NetworkState, Question, WifiState};
pub use storage::{Category, Storage, StorageCommand, Tally, categorize, tally};
pub use time::{
    TimeCommand, TimeState, follow_time, parse_manual, zone_matches, zone_offset, zone_place,
};
pub use view::SettingsView;
pub use widgets::{bucket_levels, sidebar_width};

const ENGLISH: &str = include_str!("../locales/en-US/bande-settings.ftl");

pub const TRANSLATIONS: [(&str, &str); 1] = [("en-US", ENGLISH)];
pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&TRANSLATIONS));
