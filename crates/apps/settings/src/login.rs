use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};

use bande_keyboard::{GlobalShortcut, Trigger, read_global_shortcuts, write_global_shortcuts};
use bande_l10n::text;
use bande_xdg::{
    DesktopEntry, Locale, app_languages, applications, autostart_dir, current_desktops, data_dirs,
    global_shortcuts_file, opens_at_login, set_app_language, set_open_at_login,
};

use bande_settings_panes::PaneId;

use crate::failure::pane_failed;
use crate::follow::{Update, Updates, spawn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppShortcut {
    pub app: App,
    pub id: String,
    pub description: String,
    pub keys: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoginItems {
    pub items: Vec<App>,
    pub apps: Vec<App>,
    pub languages: Vec<(App, String)>,
    pub shortcuts: Vec<AppShortcut>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginCommand {
    Refresh,
    Add(String),
    Remove(String),
    SetLanguage(String, Option<String>),
    RevokeShortcut(String, String),
}

fn sorted(mut apps: Vec<App>) -> Vec<App> {
    apps.sort_by_key(|app| app.name.to_lowercase());
    apps
}

pub fn login_items(dir: &Path, apps: &[DesktopEntry]) -> Vec<App> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let items = entries
        .flatten()
        .filter_map(|entry| {
            let id = entry.file_name().to_str()?.to_owned();
            if !id.ends_with(".desktop") || !opens_at_login(dir, &id) {
                return None;
            }
            let name = apps
                .iter()
                .find(|app| app.id == id)
                .map(|app| app.name.clone())
                .or_else(|| {
                    let text = std::fs::read_to_string(entry.path()).ok()?;
                    DesktopEntry::parse_handler(&id, &entry.path(), &text, None, &[])
                        .map(|parsed| parsed.name)
                })
                .unwrap_or_else(|| id.trim_end_matches(".desktop").to_owned());
            Some(App { id, name })
        })
        .collect();
    sorted(items)
}

pub fn login_matches(state: &LoginItems, query: &str, limit: usize) -> Vec<String> {
    let open_at_login = |app: &App| state.items.iter().any(|item| item.id == app.id);
    app_matches(&state.apps, open_at_login, query, limit)
}

pub(crate) fn app_matches(
    apps: &[App],
    excluded: impl Fn(&App) -> bool,
    query: &str,
    limit: usize,
) -> Vec<String> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut starting = Vec::new();
    let mut containing = Vec::new();
    for app in apps.iter().filter(|app| !excluded(app)) {
        let name = app.name.to_lowercase();
        if name.split_whitespace().any(|word| word.starts_with(&query)) {
            starting.push(app.id.clone());
        } else if name.contains(&query) || app.id.to_lowercase().contains(&query) {
            containing.push(app.id.clone());
        }
    }
    starting.extend(containing);
    starting.truncate(limit);
    starting
}

fn languages(apps: &[DesktopEntry]) -> Vec<(App, String)> {
    let mut chosen: Vec<(App, String)> = app_languages()
        .into_iter()
        .map(|(id, code)| {
            let name = apps
                .iter()
                .find(|app| app.app_id() == id)
                .map_or_else(|| id.clone(), |app| app.name.clone());
            let id = format!("{id}.desktop");
            (App { id, name }, code)
        })
        .collect();
    chosen.sort_by_key(|(app, _)| app.name.to_lowercase());
    chosen
}

fn read_shortcuts(file: &Path) -> std::io::Result<Vec<GlobalShortcut>> {
    match std::fs::read_to_string(file) {
        Ok(text) => Ok(read_global_shortcuts(&text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn shortcuts(apps: &[DesktopEntry]) -> Vec<AppShortcut> {
    let Some(file) = global_shortcuts_file() else {
        return Vec::new();
    };
    let bound = read_shortcuts(&file)
        .inspect_err(|error| tracing::warn!("cannot read the apps' shortcuts: {error}"))
        .unwrap_or_default();
    let mut listed: Vec<AppShortcut> = bound
        .into_iter()
        .map(|shortcut| {
            let name = apps
                .iter()
                .find(|app| app.app_id() == shortcut.app_id)
                .map_or_else(|| shortcut.app_id.clone(), |app| app.name.clone());
            AppShortcut {
                app: App {
                    id: shortcut.app_id,
                    name,
                },
                id: shortcut.id,
                description: shortcut.description,
                keys: shortcut.trigger.map(Trigger::keys),
            }
        })
        .collect();
    listed.sort_by_key(|shortcut| shortcut.app.name.to_lowercase());
    listed
}

fn revoke_shortcut(app_id: &str, id: &str) -> std::io::Result<()> {
    let file = global_shortcuts_file()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no state folder"))?;
    let kept: Vec<GlobalShortcut> = read_shortcuts(&file)?
        .into_iter()
        .filter(|shortcut| shortcut.app_id != app_id || shortcut.id != id)
        .collect();
    bande_xdg::replace_file(&file, write_global_shortcuts(&kept))
}

fn run(dir: &Path, apps: &[DesktopEntry], command: LoginCommand) -> std::io::Result<()> {
    match command {
        LoginCommand::Refresh => Ok(()),
        LoginCommand::Add(id) => match apps.iter().find(|app| app.id == id) {
            Some(app) => set_open_at_login(dir, &id, &app.path, true),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{id} is not installed"),
            )),
        },
        LoginCommand::Remove(id) => set_open_at_login(dir, &id, &PathBuf::new(), false),
        LoginCommand::SetLanguage(id, code) => {
            let code = code.map(|code| code.replace(".UTF-8", ""));
            set_app_language(id.trim_end_matches(".desktop"), code.as_deref())
        }
        LoginCommand::RevokeShortcut(app_id, id) => revoke_shortcut(&app_id, &id),
    }
}

pub fn follow_login_items(updates: Updates) -> Sender<LoginCommand> {
    let (sender, receiver) = mpsc::channel();
    spawn("login-items", move || {
        let Some(dir) = autostart_dir() else {
            return;
        };
        let load = || {
            applications(
                &data_dirs(),
                Locale::from_env().as_ref(),
                &current_desktops(),
            )
        };
        let mut apps: Option<Vec<DesktopEntry>> = None;
        for command in receiver {
            if command == LoginCommand::Refresh {
                apps = None;
            }
            let apps = apps.get_or_insert_with(load);
            let (pane, failure) = match command {
                LoginCommand::SetLanguage(..) => (
                    PaneId::LanguageTime,
                    text!("Couldn't change the app's language."),
                ),
                LoginCommand::RevokeShortcut(..) => (
                    PaneId::KeyboardShortcuts,
                    text!("Couldn't remove the app's shortcut."),
                ),
                _ => (
                    PaneId::Apps,
                    text!("Couldn't change the apps that open at login."),
                ),
            };
            let language = match &command {
                LoginCommand::SetLanguage(_, code) => code.clone(),
                _ => None,
            };
            match run(&dir, apps, command) {
                Ok(()) => {
                    if let Some(code) = language {
                        crate::locale::fetch_language_packs(code, |_| {});
                    }
                }
                Err(error) => {
                    updates.send(pane_failed(pane, failure, error));
                }
            }
            let state = LoginItems {
                items: login_items(&dir, apps),
                apps: sorted(
                    apps.iter()
                        .map(|app| App {
                            id: app.id.clone(),
                            name: app.name.clone(),
                        })
                        .collect(),
                ),
                languages: languages(apps),
                shortcuts: shortcuts(apps),
            };
            if !updates.send(Update::Login(state)) {
                break;
            }
        }
    });
    sender
}
