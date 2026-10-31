use std::collections::HashMap;
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::mimeapps::data_home;

pub const CURRENT_DESKTOP: &str = "Bande";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locale {
    lang: String,
    country: Option<String>,
    modifier: Option<String>,
}

impl Locale {
    pub fn parse(value: &str) -> Option<Self> {
        let (rest, modifier) = match value.split_once('@') {
            Some((rest, modifier)) => (rest, Some(modifier.to_owned())),
            None => (value, None),
        };
        let rest = rest.split('.').next().unwrap_or(rest);
        let (lang, country) = match rest.split_once('_') {
            Some((lang, country)) => (lang, Some(country.to_owned())),
            None => (rest, None),
        };
        if lang.is_empty() || lang == "C" || lang == "POSIX" {
            return None;
        }
        Some(Self {
            lang: lang.to_owned(),
            country,
            modifier,
        })
    }

    pub fn from_env() -> Option<Self> {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|name| env::var(name).ok())
            .find(|value| !value.is_empty())
            .and_then(|value| Self::parse(&value))
    }

    pub(crate) fn keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        if let (Some(country), Some(modifier)) = (&self.country, &self.modifier) {
            keys.push(format!("{}_{country}@{modifier}", self.lang));
        }
        if let Some(country) = &self.country {
            keys.push(format!("{}_{country}", self.lang));
        }
        if let Some(modifier) = &self.modifier {
            keys.push(format!("{}@{modifier}", self.lang));
        }
        keys.push(self.lang.clone());
        keys
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntry {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub generic_name: Option<String>,
    pub comment: Option<String>,
    pub icon: Option<String>,
    pub exec: String,
    pub working_directory: Option<PathBuf>,
    pub terminal: bool,
    pub keywords: Vec<String>,
    pub mime_types: Vec<String>,
    pub no_display: bool,
    pub startup_wm_class: Option<String>,
    pub actions: Vec<DesktopAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopAction {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub exec: String,
}

struct Group {
    values: HashMap<String, String>,
}

impl Group {
    fn raw(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    fn string(&self, key: &str) -> Option<String> {
        self.raw(key).map(unescape)
    }

    fn localized(&self, key: &str, locale: Option<&Locale>) -> Option<String> {
        locale
            .into_iter()
            .flat_map(Locale::keys)
            .find_map(|suffix| self.string(&format!("{key}[{suffix}]")))
            .or_else(|| self.string(key))
    }

    fn boolean(&self, key: &str) -> bool {
        self.raw(key) == Some("true")
    }

    fn list(&self, key: &str) -> Vec<String> {
        self.raw(key).map(split_list).unwrap_or_default()
    }

    fn localized_list(&self, key: &str, locale: Option<&Locale>) -> Vec<String> {
        locale
            .into_iter()
            .flat_map(Locale::keys)
            .find_map(|suffix| self.raw(&format!("{key}[{suffix}]")).map(split_list))
            .unwrap_or_else(|| self.list(key))
    }
}

fn unescape(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            result.push(character);
            continue;
        }
        match chars.next() {
            Some('s') => result.push(' '),
            Some('n') => result.push('\n'),
            Some('t') => result.push('\t'),
            Some('r') => result.push('\r'),
            Some('\\') => result.push('\\'),
            Some(other) => {
                result.push('\\');
                result.push(other);
            }
            None => result.push('\\'),
        }
    }
    result
}

fn split_list(value: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        match character {
            '\\' => match chars.next() {
                Some(';') => current.push(';'),
                Some(other) => {
                    current.push('\\');
                    current.push(other);
                }
                None => current.push('\\'),
            },
            ';' => items.push(unescape(&std::mem::take(&mut current))),
            other => current.push(other),
        }
    }
    if !current.is_empty() {
        items.push(unescape(&current));
    }
    items
}

fn groups(text: &str) -> HashMap<String, Group> {
    let mut groups: HashMap<String, Group> = HashMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            groups.entry(name.to_owned()).or_insert_with(|| Group {
                values: HashMap::new(),
            });
            current = Some(name.to_owned());
            continue;
        }
        if let Some(group) = current.as_ref().and_then(|name| groups.get_mut(name))
            && let Some((key, value)) = line.split_once('=')
        {
            group
                .values
                .entry(key.trim().to_owned())
                .or_insert_with(|| value.trim_start().to_owned());
        }
    }
    groups
}

fn actions(
    entry: &Group,
    groups: &HashMap<String, Group>,
    locale: Option<&Locale>,
) -> Vec<DesktopAction> {
    entry
        .list("Actions")
        .into_iter()
        .filter_map(|id| {
            let group = groups.get(&format!("Desktop Action {id}"))?;
            Some(DesktopAction {
                name: group.localized("Name", locale)?,
                exec: group.string("Exec").filter(|exec| !exec.is_empty())?,
                icon: group.string("Icon").filter(|icon| !icon.is_empty()),
                id,
            })
        })
        .collect()
}

pub fn find_executable(program: &str) -> Option<PathBuf> {
    let executable = |path: &Path| {
        fs::metadata(path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        let path = PathBuf::from(program);
        return executable(&path).then_some(path);
    }
    env::split_paths(&env::var_os("PATH")?)
        .map(|directory| directory.join(program))
        .find(|path| executable(path))
}

impl DesktopEntry {
    pub fn parse(
        id: &str,
        path: &Path,
        text: &str,
        locale: Option<&Locale>,
        desktops: &[String],
    ) -> Option<Self> {
        Self::parse_handler(id, path, text, locale, desktops).filter(|entry| !entry.no_display)
    }

    pub fn parse_handler(
        id: &str,
        path: &Path,
        text: &str,
        locale: Option<&Locale>,
        desktops: &[String],
    ) -> Option<Self> {
        let groups = groups(text);
        let group = groups.get("Desktop Entry")?;
        if group.raw("Type") != Some("Application") || group.boolean("Hidden") {
            return None;
        }
        let only = group.list("OnlyShowIn");
        if !only.is_empty() && !only.iter().any(|desktop| desktops.contains(desktop)) {
            return None;
        }
        if group
            .list("NotShowIn")
            .iter()
            .any(|desktop| desktops.contains(desktop))
        {
            return None;
        }
        if let Some(try_exec) = group.string("TryExec")
            && find_executable(&try_exec).is_none()
        {
            return None;
        }
        Some(Self {
            id: id.to_owned(),
            path: path.to_path_buf(),
            name: group.localized("Name", locale)?,
            generic_name: group.localized("GenericName", locale),
            comment: group.localized("Comment", locale),
            icon: group.string("Icon").filter(|icon| !icon.is_empty()),
            exec: group.string("Exec")?,
            working_directory: group.string("Path").map(PathBuf::from),
            terminal: group.boolean("Terminal"),
            keywords: group.localized_list("Keywords", locale),
            mime_types: group.list("MimeType"),
            no_display: group.boolean("NoDisplay"),
            startup_wm_class: group
                .string("StartupWMClass")
                .filter(|class| !class.is_empty()),
            actions: actions(group, &groups, locale),
        })
    }

    pub fn app_id(&self) -> &str {
        self.id.strip_suffix(".desktop").unwrap_or(&self.id)
    }
}

pub fn current_desktops() -> Vec<String> {
    env::var("XDG_CURRENT_DESKTOP")
        .ok()
        .filter(|value| !value.is_empty())
        .map(|value| value.split(':').map(str::to_owned).collect())
        .unwrap_or_else(|| vec![CURRENT_DESKTOP.to_owned()])
}

pub fn data_dirs() -> Vec<PathBuf> {
    let absolute = |path: &PathBuf| path.is_absolute();
    let home = data_home();
    let system = env::var_os("XDG_DATA_DIRS")
        .filter(|value| !value.is_empty())
        .map(|value| {
            env::split_paths(&value)
                .filter(absolute)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            vec![
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share"),
            ]
        });
    home.into_iter().chain(system).collect()
}

fn collect(root: &Path, directory: &Path, found: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, found);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "desktop")
            && let Ok(relative) = path.strip_prefix(root)
        {
            let id = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("-");
            found.push((id, path));
        }
    }
}

pub fn applications(
    data_dirs: &[PathBuf],
    locale: Option<&Locale>,
    desktops: &[String],
) -> Vec<DesktopEntry> {
    let mut entries = handlers(data_dirs, locale, desktops);
    entries.retain(|entry| !entry.no_display);
    entries
}

pub fn handlers(
    data_dirs: &[PathBuf],
    locale: Option<&Locale>,
    desktops: &[String],
) -> Vec<DesktopEntry> {
    let mut seen = Vec::<String>::new();
    let mut entries = Vec::new();
    for data_dir in data_dirs {
        let root = data_dir.join("applications");
        let mut found = Vec::new();
        collect(&root, &root, &mut found);
        for (id, path) in found {
            if seen.contains(&id) {
                continue;
            }
            seen.push(id.clone());
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            if let Some(entry) = DesktopEntry::parse_handler(&id, &path, &text, locale, desktops) {
                entries.push(entry);
            }
        }
    }
    entries.sort_by_cached_key(|entry| (entry.name.to_lowercase(), entry.id.clone()));
    entries
}
