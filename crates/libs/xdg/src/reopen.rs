use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::entry::{Locale, applications, current_desktops, data_dirs};
use crate::identity::find_app;

const REOPEN_FILE: &str = "bande/reopen";
const RUNNING_FILE: &str = "bande/running";
const CRASHED_FILE: &str = "bande/crashed";
const RESUME_FOLDER: &str = "bande/resume";

fn state_file(name: &str) -> Option<PathBuf> {
    crate::state_home().map(|state| state.join(name))
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
        let _ = fs::remove_file(crashed);
        return fs::remove_file(running);
    }
    fs::rename(running, crashed)
}

pub fn running_apps_contents(apps: &[String]) -> String {
    let mut listed = apps.join("\n");
    listed.push('\n');
    listed
}

pub fn remember_running_apps(file: &Path, running: &[String]) {
    if let Err(error) = crate::replace_file(file, running_apps_contents(running)) {
        tracing::warn!("cannot remember the apps to reopen: {error}");
    }
}

pub fn reopen_listed_apps(apps: &[String], mut reopening: impl FnMut(&str)) {
    let entries = applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    );
    let mut launched = Vec::new();
    for app in apps {
        let Some(entry) = find_app(&entries, app) else {
            continue;
        };
        reopening(app);
        if launched.contains(&entry.app_id()) {
            continue;
        }
        launched.push(entry.app_id());
        if let Err(error) = crate::launch(entry, None) {
            tracing::warn!("cannot reopen {app}: {error}");
        }
    }
}

pub fn listed_apps(file: &Path) -> Vec<String> {
    fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|app| !app.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn forget_saved_windows(apps: &[String]) {
    let Some(state) = crate::state_home() else {
        return;
    };
    for app in apps {
        let file = state.join(RESUME_FOLDER).join(app);
        if let Err(error) = fs::remove_file(&file)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!("cannot forget the windows of {app}: {error}");
        }
    }
}

pub fn take_apps_to_reopen(file: &Path) -> Vec<String> {
    let listed = listed_apps(file);
    if let Err(error) = fs::remove_file(file)
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!("cannot forget the apps to reopen: {error}");
    }
    listed
}
