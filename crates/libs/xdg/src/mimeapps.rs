use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::entry::DesktopEntry;

const LIST_NAME: &str = "mimeapps.list";
const DEFAULTS: &str = "Default Applications";
const ADDED: &str = "Added Associations";
const REMOVED: &str = "Removed Associations";

fn base_dir(variable: &str, fallback: &str) -> Option<PathBuf> {
    env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))
}

pub fn config_home() -> Option<PathBuf> {
    base_dir("XDG_CONFIG_HOME", ".config")
}

pub fn data_home() -> Option<PathBuf> {
    base_dir("XDG_DATA_HOME", ".local/share")
}

pub fn cache_home() -> Option<PathBuf> {
    base_dir("XDG_CACHE_HOME", ".cache")
}

pub fn state_home() -> Option<PathBuf> {
    base_dir("XDG_STATE_HOME", ".local/state")
}

pub fn config_dirs() -> Vec<PathBuf> {
    let system = env::var_os("XDG_CONFIG_DIRS")
        .filter(|value| !value.is_empty())
        .map(|value| {
            env::split_paths(&value)
                .filter(|path| path.is_absolute())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![PathBuf::from("/etc/xdg")]);
    config_home().into_iter().chain(system).collect()
}

pub fn mimeapps_paths(
    config_dirs: &[PathBuf],
    data_dirs: &[PathBuf],
    desktops: &[String],
) -> Vec<PathBuf> {
    let names = desktops
        .iter()
        .map(|desktop| format!("{}-{LIST_NAME}", desktop.to_lowercase()))
        .chain([LIST_NAME.to_owned()])
        .collect::<Vec<_>>();
    config_dirs
        .iter()
        .cloned()
        .chain(data_dirs.iter().map(|dir| dir.join("applications")))
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .collect()
}

#[derive(Debug, Default)]
struct List {
    groups: HashMap<String, HashMap<String, Vec<String>>>,
}

impl List {
    fn parse(text: &str) -> Self {
        let mut groups: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();
        let mut current = None;
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                current = Some(name.to_owned());
            } else if let (Some(group), Some((key, value))) = (&current, line.split_once('=')) {
                let apps = value
                    .split(';')
                    .map(str::trim)
                    .filter(|app| !app.is_empty())
                    .map(str::to_owned)
                    .collect();
                groups
                    .entry(group.clone())
                    .or_default()
                    .entry(key.trim().to_owned())
                    .or_insert(apps);
            }
        }
        Self { groups }
    }

    fn apps(&self, group: &str, mime: &str) -> &[String] {
        self.groups
            .get(group)
            .and_then(|values| values.get(mime))
            .map_or(&[], Vec::as_slice)
    }
}

#[derive(Debug, Default)]
pub struct MimeApps {
    lists: Vec<List>,
}

impl MimeApps {
    pub fn load(paths: &[PathBuf]) -> Self {
        Self {
            lists: paths
                .iter()
                .filter_map(|path| fs::read_to_string(path).ok())
                .map(|text| List::parse(&text))
                .collect(),
        }
    }

    fn associated<'a>(&self, mime: &str, handlers: &'a [DesktopEntry]) -> Vec<&'a DesktopEntry> {
        let find = |id: &str| handlers.iter().find(|entry| entry.id == id);
        let mut removed = HashSet::new();
        let mut found: Vec<&DesktopEntry> = Vec::new();
        for list in &self.lists {
            for id in list.apps(ADDED, mime) {
                if !removed.contains(id.as_str())
                    && let Some(entry) = find(id)
                    && !found.iter().any(|known| known.id == entry.id)
                {
                    found.push(entry);
                }
            }
            removed.extend(list.apps(REMOVED, mime).iter().map(String::as_str));
        }
        for entry in handlers {
            if entry.mime_types.iter().any(|known| known == mime)
                && !removed.contains(entry.id.as_str())
                && !found.iter().any(|known| known.id == entry.id)
            {
                found.push(entry);
            }
        }
        found
    }

    fn default_of<'a>(&self, mime: &str, handlers: &'a [DesktopEntry]) -> Option<&'a DesktopEntry> {
        self.lists
            .iter()
            .flat_map(|list| list.apps(DEFAULTS, mime))
            .find_map(|id| handlers.iter().find(|entry| entry.id == *id))
            .or_else(|| self.associated(mime, handlers).into_iter().next())
    }

    pub fn default_for<'a>(
        &self,
        ancestry: &[String],
        handlers: &'a [DesktopEntry],
    ) -> Option<&'a DesktopEntry> {
        ancestry
            .iter()
            .find_map(|mime| self.default_of(mime, handlers))
    }

    pub fn apps_for<'a>(
        &self,
        ancestry: &[String],
        handlers: &'a [DesktopEntry],
    ) -> Vec<&'a DesktopEntry> {
        let mut apps: Vec<&DesktopEntry> =
            self.default_for(ancestry, handlers).into_iter().collect();
        for entry in ancestry
            .iter()
            .flat_map(|mime| self.associated(mime, handlers))
        {
            if !apps.iter().any(|known| known.id == entry.id) {
                apps.push(entry);
            }
        }
        apps
    }
}

fn set_key(text: &str, group: &str, key: &str, value: &str) -> String {
    let mut lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
    let header = format!("[{group}]");
    let entry = format!("{key}={value}");
    let Some(start) = lines.iter().position(|line| line.trim() == header) else {
        if lines.last().is_some_and(|line| !line.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(header);
        lines.push(entry);
        return lines.join("\n") + "\n";
    };
    let end = lines[start + 1..]
        .iter()
        .position(|line| line.trim_start().starts_with('['))
        .map_or(lines.len(), |offset| start + 1 + offset);
    let existing = lines[start + 1..end].iter().position(|line| {
        line.split_once('=')
            .is_some_and(|(name, _)| name.trim() == key)
    });
    match existing {
        Some(offset) => lines[start + 1 + offset] = entry,
        None => {
            let last = lines[start + 1..end]
                .iter()
                .rposition(|line| !line.trim().is_empty())
                .map_or(start + 1, |offset| start + 2 + offset);
            lines.insert(last, entry);
        }
    }
    lines.join("\n") + "\n"
}

pub fn set_default_app(list: &Path, mime: &str, desktop_id: &str) -> io::Result<()> {
    let text = match fs::read_to_string(list) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let added = List::parse(&text);
    let mut associations = vec![desktop_id.to_owned()];
    associations.extend(
        added
            .apps(ADDED, mime)
            .iter()
            .filter(|id| *id != desktop_id)
            .cloned(),
    );
    let text = set_key(&text, DEFAULTS, mime, &format!("{desktop_id};"));
    let text = set_key(&text, ADDED, mime, &(associations.join(";") + ";"));
    crate::replace_file(list, &text)
}
