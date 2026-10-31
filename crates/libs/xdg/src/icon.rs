use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

pub const FALLBACK_THEME: &str = "hicolor";
pub const ICON_THEMES: [&str; 2] = ["Bande", "Adwaita"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Fixed,
    Scalable,
    Threshold,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Directory {
    path: String,
    size: u32,
    scale: u32,
    kind: Kind,
    min_size: u32,
    max_size: u32,
    threshold: u32,
}

impl Directory {
    fn matches(&self, size: u32, scale: u32) -> bool {
        if self.scale != scale {
            return false;
        }
        match self.kind {
            Kind::Fixed => self.size == size,
            Kind::Scalable => (self.min_size..=self.max_size).contains(&size),
            Kind::Threshold => {
                self.size.saturating_sub(self.threshold) <= size
                    && size <= self.size + self.threshold
            }
        }
    }

    fn distance(&self, size: u32, scale: u32) -> u32 {
        let wanted = size * scale;
        let (low, high) = match self.kind {
            Kind::Fixed => (self.size * self.scale, self.size * self.scale),
            Kind::Scalable => (self.min_size * self.scale, self.max_size * self.scale),
            Kind::Threshold => (
                self.size.saturating_sub(self.threshold) * self.scale,
                (self.size + self.threshold) * self.scale,
            ),
        };
        if wanted < low {
            low - wanted
        } else {
            wanted.saturating_sub(high)
        }
    }
}

struct Theme {
    directories: Vec<Directory>,
    inherits: Vec<String>,
}

fn parse_index(text: &str) -> Theme {
    let mut groups: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            groups.push((name.to_owned(), Vec::new()));
        } else if let (Some((key, value)), Some((_, values))) =
            (line.split_once('='), groups.last_mut())
        {
            values.push((key.trim().to_owned(), value.trim().to_owned()));
        }
    }
    let value = |group: &str, key: &str| {
        groups
            .iter()
            .find(|(name, _)| name == group)
            .and_then(|(_, values)| values.iter().find(|(name, _)| name == key))
            .map(|(_, value)| value.clone())
    };
    let list = |group: &str, key: &str| {
        value(group, key)
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let mut names = list("Icon Theme", "Directories");
    names.extend(list("Icon Theme", "ScaledDirectories"));
    let directories = names
        .into_iter()
        .filter_map(|path| {
            let number = |key: &str| value(&path, key).and_then(|value| value.parse::<u32>().ok());
            let size = number("Size")?;
            let kind = match value(&path, "Type").as_deref() {
                Some("Fixed") => Kind::Fixed,
                Some("Scalable") => Kind::Scalable,
                _ => Kind::Threshold,
            };
            Some(Directory {
                scale: number("Scale").unwrap_or(1),
                min_size: number("MinSize").unwrap_or(size),
                max_size: number("MaxSize").unwrap_or(size),
                threshold: number("Threshold").unwrap_or(2),
                path,
                size,
                kind,
            })
        })
        .collect();
    Theme {
        directories,
        inherits: list("Icon Theme", "Inherits"),
    }
}

const EXTENSIONS: [&str; 2] = ["png", "svg"];
const RECHECK: Duration = Duration::from_secs(2);

static LOOKUP: LazyLock<Mutex<IconLookup>> = LazyLock::new(|| Mutex::new(IconLookup::new(RECHECK)));

struct Fresh<T> {
    value: Arc<T>,
    modified: Option<SystemTime>,
    checked: Instant,
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

fn fresh<T: Default>(
    cache: &mut HashMap<PathBuf, Fresh<T>>,
    recheck: Duration,
    path: &Path,
    load: impl FnOnce(&Path) -> T,
) -> Arc<T> {
    let now = Instant::now();
    if let Some(known) = cache.get(path)
        && now.duration_since(known.checked) < recheck
    {
        return Arc::clone(&known.value);
    }
    let modified = modified(path);
    if let Some(known) = cache.get_mut(path)
        && known.modified == modified
    {
        known.checked = now;
        return Arc::clone(&known.value);
    }
    let value = Arc::new(modified.map_or_else(T::default, |_| load(path)));
    cache.insert(
        path.to_path_buf(),
        Fresh {
            value: Arc::clone(&value),
            modified,
            checked: now,
        },
    );
    value
}

fn file_names(folder: &Path) -> HashSet<OsString> {
    fs::read_dir(folder)
        .map(|entries| entries.flatten().map(|entry| entry.file_name()).collect())
        .unwrap_or_default()
}

struct IconLookup {
    recheck: Duration,
    themes: HashMap<PathBuf, Fresh<Option<Theme>>>,
    folders: HashMap<PathBuf, Fresh<HashSet<OsString>>>,
}

impl IconLookup {
    fn new(recheck: Duration) -> Self {
        Self {
            recheck,
            themes: HashMap::new(),
            folders: HashMap::new(),
        }
    }

    fn theme(&mut self, name: &str, base_dirs: &[PathBuf]) -> Option<Arc<Option<Theme>>> {
        base_dirs.iter().find_map(|base| {
            let index = base.join(name).join("index.theme");
            let theme = fresh(&mut self.themes, self.recheck, &index, |path| {
                fs::read_to_string(path).ok().map(|text| parse_index(&text))
            });
            theme.is_some().then_some(theme)
        })
    }

    fn file(&mut self, folder: PathBuf, icon: &str) -> Option<PathBuf> {
        let names = fresh(&mut self.folders, self.recheck, &folder, file_names);
        EXTENSIONS
            .iter()
            .map(|extension| format!("{icon}.{extension}"))
            .find(|name| names.contains(OsStr::new(name)))
            .map(|name| folder.join(name))
    }

    fn in_directory(
        &mut self,
        theme_name: &str,
        directory: &Directory,
        icon: &str,
        base_dirs: &[PathBuf],
    ) -> Option<PathBuf> {
        base_dirs
            .iter()
            .find_map(|base| self.file(base.join(theme_name).join(&directory.path), icon))
    }

    fn lookup_in(
        &mut self,
        theme_name: &str,
        theme: &Theme,
        icon: &str,
        size: u32,
        base_dirs: &[PathBuf],
    ) -> Option<PathBuf> {
        for directory in theme
            .directories
            .iter()
            .filter(|directory| directory.matches(size, 1))
        {
            if let Some(found) = self.in_directory(theme_name, directory, icon, base_dirs) {
                return Some(found);
            }
        }
        let mut closest: Option<(u32, PathBuf)> = None;
        for directory in &theme.directories {
            let distance = directory.distance(size, 1);
            if closest.as_ref().is_some_and(|(best, _)| *best <= distance) {
                continue;
            }
            if let Some(found) = self.in_directory(theme_name, directory, icon, base_dirs) {
                closest = Some((distance, found));
            }
        }
        closest.map(|(_, path)| path)
    }

    fn in_theme(
        &mut self,
        name: &str,
        icon: &str,
        size: u32,
        base_dirs: &[PathBuf],
    ) -> Option<Result<PathBuf, Vec<String>>> {
        let theme = self.theme(name, base_dirs)?;
        let theme = theme.as_ref().as_ref()?;
        Some(
            self.lookup_in(name, theme, icon, size, base_dirs)
                .ok_or_else(|| theme.inherits.clone()),
        )
    }

    fn find(
        &mut self,
        icon: &str,
        size: u32,
        themes: &[&str],
        data_dirs: &[PathBuf],
    ) -> Option<PathBuf> {
        let path = Path::new(icon);
        if path.is_absolute() {
            return path.is_file().then(|| path.to_path_buf());
        }
        let base_dirs = icon_dirs(data_dirs);
        let mut pending: VecDeque<String> =
            themes.iter().map(|theme| (*theme).to_owned()).collect();
        let mut visited = Vec::new();
        while let Some(name) = pending.pop_front() {
            if visited.contains(&name) {
                continue;
            }
            match self.in_theme(&name, icon, size, &base_dirs) {
                Some(Ok(found)) => return Some(found),
                Some(Err(inherits)) => pending.extend(inherits),
                None => {}
            }
            visited.push(name);
        }
        if !visited.iter().any(|name| name == FALLBACK_THEME)
            && let Some(Ok(found)) = self.in_theme(FALLBACK_THEME, icon, size, &base_dirs)
        {
            return Some(found);
        }
        data_dirs
            .iter()
            .find_map(|dir| self.file(dir.join("pixmaps"), icon))
    }
}

pub fn icon_dirs(data_dirs: &[PathBuf]) -> Vec<PathBuf> {
    data_dirs.iter().map(|dir| dir.join("icons")).collect()
}

pub fn find_icon(icon: &str, size: u32, themes: &[&str], data_dirs: &[PathBuf]) -> Option<PathBuf> {
    LOOKUP
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .find(icon, size, themes, data_dirs)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::Duration;

    use super::IconLookup;

    const THEME: &str =
        "[Icon Theme]\nName=Plain\nDirectories=48x48/apps\n\n[48x48/apps]\nSize=48\nType=Fixed\n";

    fn install(icons: &Path, name: &str) {
        fs::write(icons.join("Plain/48x48/apps").join(name), "").unwrap();
    }

    #[test]
    fn an_icon_installed_later_is_found_once_its_folder_is_checked_again() {
        let data = tempfile::tempdir().unwrap();
        let icons = data.path().join("icons");
        fs::create_dir_all(icons.join("Plain/48x48/apps")).unwrap();
        fs::write(icons.join("Plain/index.theme"), THEME).unwrap();
        install(&icons, "editor.png");
        let dirs = [data.path().to_path_buf()];
        let mut remembering = IconLookup::new(Duration::from_secs(3600));
        let mut checking = IconLookup::new(Duration::ZERO);
        for lookup in [&mut remembering, &mut checking] {
            assert!(lookup.find("editor", 48, &["Plain"], &dirs).is_some());
            assert_eq!(lookup.find("viewer", 48, &["Plain"], &dirs), None);
        }
        std::thread::sleep(Duration::from_millis(20));
        install(&icons, "viewer.svg");
        assert_eq!(remembering.find("viewer", 48, &["Plain"], &dirs), None);
        assert_eq!(
            checking.find("viewer", 48, &["Plain"], &dirs),
            Some(icons.join("Plain/48x48/apps/viewer.svg"))
        );
    }
}
