mod apps;
mod autostart;
mod dirs;
mod entry;
mod icon;
mod identity;
mod launch;
mod mime;
mod mimeapps;
mod uri;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

const ENGLISH: &str = include_str!("../locales/en-US/unbound-xdg.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub use apps::Apps;
pub use autostart::{autostart_dir, opens_at_login, set_open_at_login};
pub use dirs::{UserDir, user_dir};
pub use entry::{
    CURRENT_DESKTOP, DesktopAction, DesktopEntry, Locale, applications, current_desktops,
    data_dirs, find_executable, handlers,
};
pub use icon::{FALLBACK_THEME, ICON_THEMES, find_icon};
pub use identity::{app_name, find_app, running_app};
pub use launch::{
    ACTIVATION_TOKEN, ExecError, LaunchError, TERMINAL, launch, launch_action, launch_with,
    launch_with_arguments, spawn_detached,
};
pub use mime::{DIRECTORY, MimeDatabase, OCTET_STREAM, TEXT_PLAIN};
pub use mimeapps::{
    MimeApps, cache_home, config_dirs, config_home, data_home, mimeapps_paths, set_default_app,
    state_home,
};
pub use uri::{file_uri, path_from_file_uri, percent_decode, percent_encode};
