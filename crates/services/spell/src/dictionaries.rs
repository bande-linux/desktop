use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use spellbook::Dictionary;

pub(crate) fn language_data(name: &str) -> HashMap<String, String> {
    let mut merged = HashMap::new();
    for directory in data_directories().iter().rev() {
        let Ok(contents) = fs::read_to_string(directory.join("bande/text").join(name)) else {
            continue;
        };
        for line in contents.lines() {
            if let Some((key, value)) = line.split_once('=') {
                merged.insert(key.trim().to_owned(), value.trim().to_owned());
            }
        }
    }
    merged
}

pub(crate) fn input_source_language(layout: &str, table: &HashMap<String, String>) -> String {
    table
        .get(layout)
        .cloned()
        .unwrap_or_else(|| layout.to_owned())
}

pub(crate) fn data_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(home) = bande_data_home() {
        directories.push(home);
    }
    let shared = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    directories.extend(shared.split(':').map(PathBuf::from));
    directories
}

pub(crate) fn bande_data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|home| home.join(".local/share")))
}

pub(crate) fn data_stamp() -> Vec<Option<SystemTime>> {
    data_directories()
        .iter()
        .flat_map(|directory| [directory.join("hunspell"), directory.join("bande/text")])
        .map(|path| fs::metadata(path).and_then(|metadata| metadata.modified()).ok())
        .collect()
}

pub(crate) fn installed() -> BTreeMap<String, PathBuf> {
    let mut found = BTreeMap::new();
    for directory in data_directories() {
        let Ok(entries) = fs::read_dir(directory.join("hunspell")) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|extension| extension == "aff")
                && path.with_extension("dic").is_file()
                && let Some(name) = path.file_stem().and_then(|stem| stem.to_str())
            {
                found.entry(name.to_owned()).or_insert(path);
            }
        }
    }
    found
}

pub(crate) fn language_of(dictionary: &str) -> &str {
    dictionary.split(['_', '-']).next().unwrap_or(dictionary)
}

pub(crate) fn best_for(
    wanted: &str,
    region: Option<&str>,
    installed: &BTreeMap<String, PathBuf>,
) -> Option<String> {
    if installed.contains_key(wanted) {
        return Some(wanted.to_owned());
    }
    let language = language_of(wanted);
    let candidates = [
        region.map(|region| format!("{language}_{region}")),
        language_data(language).remove("dictionary"),
        Some(format!("{language}_{}", language.to_uppercase())),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|name| installed.contains_key(name))
        .or_else(|| {
            installed
                .keys()
                .find(|name| language_of(name) == language)
                .cloned()
        })
}

pub(crate) fn load(aff: &Path) -> Option<Dictionary> {
    let affix = fs::read_to_string(aff).ok()?;
    let words = fs::read_to_string(aff.with_extension("dic")).ok()?;
    match Dictionary::new(&affix, &words) {
        Ok(dictionary) => Some(dictionary),
        Err(error) => {
            tracing::warn!("cannot read dictionary {}: {error}", aff.display());
            None
        }
    }
}
