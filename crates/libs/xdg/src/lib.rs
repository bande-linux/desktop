mod apps;
mod dirs;
mod entry;
mod icon;
mod launch;
mod mime;
mod mimeapps;
mod uri;

pub use apps::Apps;
pub use dirs::{UserDir, user_dir};
pub use entry::{
    CURRENT_DESKTOP, DesktopAction, DesktopEntry, Locale, applications, current_desktops,
    data_dirs, handlers,
};
pub use icon::{FALLBACK_THEME, ICON_THEMES, find_icon};
pub use launch::{
    ACTIVATION_TOKEN, ExecError, LaunchError, TERMINAL, launch, launch_action, launch_with,
    spawn_detached,
};
pub use mime::{DIRECTORY, MimeDatabase, OCTET_STREAM, TEXT_PLAIN};
pub use mimeapps::{MimeApps, config_dirs, config_home, mimeapps_paths, set_default_app};
pub use uri::{file_uri, path_from_file_uri, percent_decode, percent_encode};
