mod about;
mod arrangement;
mod backup;
mod battery;
mod brightness;
mod client;
mod clock;
mod consent;
mod crash_reports;
mod crypt;
mod defaults;
mod displays;
mod failure;
mod follow;
mod forget;
mod general;
mod header;
mod language;
mod locale;
mod login;
mod network;
mod night;
mod notifications;
mod pages;
mod passwords;
mod permissions;
mod pictures;
mod pointing;
mod power;
mod printers;
mod privacy;
mod privilege;
mod security_status;
mod services;
mod sharing;
mod software;
mod spelling;
mod state;
mod storage;
mod time;
mod updates;
mod users;
mod view;
mod widgets;
mod wifi;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use about::{About, AboutCommand, chip_name, memory_bytes, static_hostname, system_version};
pub use backup::{BackupCommand, BackupUpdate};
pub use bande_settings_panes::{PaneId, SettingId, Value};
pub use battery::{BatteryCommand, BatteryState, Charge, charge_of, follow_battery};
pub use client::{CONFIG_SETTINGS, SettingsClient};
pub use defaults::{DefaultApp, DefaultAppCommand, choose_default, default_apps};
pub use displays::Displays;
pub use follow::{Command, Feed, Services, Update, Updates, feed, follow_config, worker};
pub use locale::{
    AvailableLocale, LocaleCommand, LocaleState, assignments, parse_assignments, supported_locales,
};
pub use login::{App, LoginCommand, LoginItems, login_items, login_matches};
pub use night::{clock_text, night_summary};
pub use notifications::{AppNotifications, NotificationCommand, follow_app_notifications};
pub use passwords::{Concealed, PasswordCommand, Saved, follow_passwords};
pub use permissions::{Grant, Grants, PermissionCommand, follow_permissions};
pub use pictures::{Pair, PictureCommand, bundled_pairs, thumbnail};
pub use pointing::{PointingDevices, UDEV_DATA, follow_pointing};
pub use power::charge_text;
pub use privilege::{Privilege, refused};
pub use services::{
    AudioCommand, BluetoothCommand, NetworkCommand, follow_bluetooth, follow_network,
};
pub use software::{
    Need, Package, Reboot, Refresh, SoftwareCommand, SoftwareState, UpdateNotice, announce_log_out,
    announce_updates, check, finish_update, follow_software, open_updates, update,
};
pub use state::{Answer, BluetoothState, Display, Joined, Mode, NetworkState, Question, WifiState};
pub use storage::{Category, Storage, StorageCommand, Tally, categorize, tally};
pub use time::{
    TimeCommand, TimeState, follow_time, parse_manual, zone_matches, zone_offset, zone_place,
};
pub use view::SettingsView;
pub use widgets::{bucket_levels, sidebar_width};

pub const TRANSLATIONS: &[(&str, &str)] = bande_l10n::translations!("bande-settings");
pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(TRANSLATIONS));
