use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};

use unbound_xdg::{
    DesktopEntry, Locale, applications, autostart_dir, current_desktops, data_dirs, opens_at_login,
    set_open_at_login,
};

use crate::failure::unexpected;
use crate::follow::{Update, Updates, spawn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoginItems {
    pub items: Vec<App>,
    pub apps: Vec<App>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginCommand {
    Refresh,
    Add(String),
    Remove(String),
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
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let open = |app: &&App| !state.items.iter().any(|item| item.id == app.id);
    let mut starting = Vec::new();
    let mut containing = Vec::new();
    for app in state.apps.iter().filter(open) {
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
    }
}

pub fn follow_login_items(updates: Updates) -> Sender<LoginCommand> {
    let (sender, receiver) = mpsc::channel();
    spawn("login-items", move || {
        let config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
        let Some(dir) = autostart_dir(config_home.as_deref(), std::env::home_dir().as_deref())
        else {
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
            if let Err(error) = run(&dir, apps, command) {
                updates.send(unexpected(error));
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
            };
            if !updates.send(Update::Login(state)) {
                break;
            }
        }
    });
    sender
}
