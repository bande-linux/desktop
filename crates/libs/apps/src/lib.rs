mod entry;
mod icon;
mod launch;

pub use entry::{CURRENT_DESKTOP, DesktopEntry, Locale, applications, current_desktops, data_dirs};
pub use icon::{FALLBACK_THEME, find_icon, load_icon};
pub use launch::{ACTIVATION_TOKEN, ExecError, LaunchError, TERMINAL, launch, spawn_detached};
