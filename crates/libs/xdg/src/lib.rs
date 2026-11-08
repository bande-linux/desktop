mod app_languages;
mod apps;
mod autostart;
mod dirs;
mod entry;
mod file;
mod icon;
mod identity;
mod installed;
mod launch;
mod mime;
mod mimeapps;
mod reopen;
mod units;
mod uri;

use std::sync::LazyLock;

use bande_l10n::Localizer;

static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-xdg")));

pub use app_languages::{app_language, app_languages, set_app_language};
pub use apps::{Apps, FileTypes, OpenWithChoice};
pub use autostart::{autostart_dir, opens_at_login, set_open_at_login};
pub use dirs::{UserDir, read_user_dirs, user_dir, user_folder};
pub use entry::{
    CURRENT_DESKTOP, DesktopAction, DesktopEntry, Locale, applications, current_desktops,
    data_dirs, find_executable, handlers, own_code_replaced, own_executable, runs_replaced_code,
};
pub use file::{replace_file, replace_private_file, sync_folder};
pub use icon::{FALLBACK_THEME, ICON_THEMES, find_icon};
pub use identity::{
    app_for_identity, app_name, find_app, find_app_for_process, notifying_app, notifying_app_name,
    running_app, write_launcher,
};
pub use installed::{InstallWatch, is_installed};
pub use launch::{
    ACTIVATION_TOKEN, ExecError, LaunchError, OpenError, TERMINAL, TERMINAL_APP_ID,
    follow_oom_kills, launch, launch_action, launch_with, launch_with_arguments, open,
    spawn_detached, wait_for_exit,
};
pub use mime::{DIRECTORY, MimeDatabase, OCTET_STREAM, TEXT_PLAIN};
pub use mimeapps::{
    MimeApps, cache_home, config_dirs, config_home, data_home, mimeapps_paths, set_default_app,
    state_home,
};
pub use reopen::{
    crashed_apps_file, forget_saved_windows, listed_apps, remember_running_apps, reopen_file,
    reopen_listed_apps, running_apps_contents, running_apps_file, set_aside_apps_of_unclean_end,
    take_apps_to_reopen,
};
pub use units::unit_app_ids;
pub use uri::{file_uri, path_from_file_uri, percent_decode, percent_encode};
