use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::entry::{DesktopEntry, Locale, applications, current_desktops, data_dirs};
use crate::identity::find_app;

const REOPEN_FILE: &str = "bande/reopen";
const RUNNING_FILE: &str = "bande/running";
const CRASHED_FILE: &str = "bande/crashed";

fn state_file(name: &str) -> Option<PathBuf> {
    env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .map(|state| state.join(name))
}

pub fn reopen_file() -> Option<PathBuf> {
    state_file(REOPEN_FILE)
}

pub fn running_apps_file() -> Option<PathBuf> {
    state_file(RUNNING_FILE)
}

pub fn crashed_apps_file() -> Option<PathBuf> {
    state_file(CRASHED_FILE)
}

pub fn set_aside_apps_of_unclean_end(
    running: &Path,
    reopen: &Path,
    crashed: &Path,
) -> io::Result<()> {
    if !running.exists() {
        return Ok(());
    }
    if reopen.exists() {
        return fs::remove_file(running);
    }
    fs::rename(running, crashed)
}

pub fn write_running_apps(file: &Path, apps: &[String]) -> io::Result<()> {
    let mut listed = apps.join("\n");
    listed.push('\n');
    crate::replace_file(file, &listed)
}

pub fn apps_to_reopen<'a>(entries: &'a [DesktopEntry], running: &[String]) -> Vec<&'a str> {
    let mut apps = Vec::new();
    for window_app in running {
        if let Some(entry) = find_app(entries, window_app)
            && !apps.contains(&entry.app_id())
        {
            apps.push(entry.app_id());
        }
    }
    apps
}

pub fn remember_apps(file: &Path, apps: &[&str]) -> io::Result<()> {
    if let Some(directory) = file.parent() {
        fs::create_dir_all(directory)?;
    }
    let mut listed = apps.join("\n");
    listed.push('\n');
    fs::write(file, listed)
}

pub fn remember_running_apps(file: &Path, running: &[String]) {
    let entries = applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    );
    if let Err(error) = remember_apps(file, &apps_to_reopen(&entries, running)) {
        tracing::warn!("cannot remember the apps to reopen: {error}");
    }
}

pub fn take_apps_to_reopen(file: &Path) -> Vec<String> {
    let listed = fs::read_to_string(file).unwrap_or_default();
    if let Err(error) = fs::remove_file(file)
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!("cannot forget the apps to reopen: {error}");
    }
    listed
        .lines()
        .map(str::trim)
        .filter(|app| !app.is_empty())
        .map(str::to_owned)
        .collect()
}
