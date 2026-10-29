use std::fs;
use std::path::{Path, PathBuf};

pub const FALLBACK_THEME: &str = "hicolor";

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

fn theme(name: &str, base_dirs: &[PathBuf]) -> Option<Theme> {
    base_dirs
        .iter()
        .find_map(|base| fs::read_to_string(base.join(name).join("index.theme")).ok())
        .map(|text| parse_index(&text))
}

fn lookup_in(
    theme_name: &str,
    theme: &Theme,
    icon: &str,
    size: u32,
    base_dirs: &[PathBuf],
) -> Option<PathBuf> {
    let candidates = |directory: &Directory| -> Vec<PathBuf> {
        base_dirs
            .iter()
            .flat_map(|base| {
                EXTENSIONS.iter().map(move |extension| {
                    base.join(theme_name)
                        .join(&directory.path)
                        .join(format!("{icon}.{extension}"))
                })
            })
            .collect()
    };
    if let Some(found) = theme
        .directories
        .iter()
        .filter(|directory| directory.matches(size, 1))
        .flat_map(candidates)
        .find(|path| path.is_file())
    {
        return Some(found);
    }
    theme
        .directories
        .iter()
        .flat_map(|directory| {
            candidates(directory)
                .into_iter()
                .map(move |path| (directory, path))
        })
        .filter(|(_, path)| path.is_file())
        .min_by_key(|(directory, _)| directory.distance(size, 1))
        .map(|(_, path)| path)
}

pub fn icon_dirs(data_dirs: &[PathBuf]) -> Vec<PathBuf> {
    data_dirs.iter().map(|dir| dir.join("icons")).collect()
}

pub fn find_icon(icon: &str, size: u32, themes: &[&str], data_dirs: &[PathBuf]) -> Option<PathBuf> {
    let path = Path::new(icon);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    let base_dirs = icon_dirs(data_dirs);
    let mut pending: Vec<String> = themes.iter().map(|theme| (*theme).to_owned()).collect();
    let mut visited = Vec::new();
    while !pending.is_empty() {
        let name = pending.remove(0);
        if visited.contains(&name) {
            continue;
        }
        visited.push(name.clone());
        let Some(theme) = theme(&name, &base_dirs) else {
            continue;
        };
        if let Some(found) = lookup_in(&name, &theme, icon, size, &base_dirs) {
            return Some(found);
        }
        pending.extend(theme.inherits);
    }
    if !visited.iter().any(|name| name == FALLBACK_THEME)
        && let Some(theme) = theme(FALLBACK_THEME, &base_dirs)
        && let Some(found) = lookup_in(FALLBACK_THEME, &theme, icon, size, &base_dirs)
    {
        return Some(found);
    }
    data_dirs
        .iter()
        .flat_map(|dir| {
            EXTENSIONS
                .iter()
                .map(move |extension| dir.join("pixmaps").join(format!("{icon}.{extension}")))
        })
        .find(|path| path.is_file())
}
