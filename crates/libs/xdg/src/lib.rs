mod dirs;
mod entry;
mod icon;
mod launch;

pub use dirs::{UserDir, user_dir};
pub use entry::{CURRENT_DESKTOP, DesktopEntry, Locale, applications, current_desktops, data_dirs};
pub use icon::{FALLBACK_THEME, find_icon};
pub use launch::{ACTIVATION_TOKEN, ExecError, LaunchError, TERMINAL, launch, spawn_detached};
