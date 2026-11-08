use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) struct Store {
    settings_path: PathBuf,
    learned_path: PathBuf,
    replacements_path: PathBuf,
    pub(crate) options: BTreeMap<String, bool>,
    pub(crate) languages: Vec<String>,
    pub(crate) learned: BTreeSet<String>,
}

const LANGUAGES_KEY: &str = "languages";

impl Store {
    pub(crate) fn open(config: &Path, data: &Path) -> Self {
        let settings_path = config.join("bande/spelling");
        let learned_path = data.join("bande/learned-words");
        let mut options = BTreeMap::new();
        let mut languages = Vec::new();
        for line in fs::read_to_string(&settings_path)
            .unwrap_or_default()
            .lines()
        {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match (key.trim(), value.trim()) {
                (LANGUAGES_KEY, value) => {
                    languages = value.split_whitespace().map(str::to_owned).collect();
                }
                (key, "true") => {
                    options.insert(key.to_owned(), true);
                }
                (key, "false") => {
                    options.insert(key.to_owned(), false);
                }
                _ => {}
            }
        }
        let learned = fs::read_to_string(&learned_path)
            .unwrap_or_default()
            .lines()
            .filter(|word| !word.is_empty())
            .map(str::to_owned)
            .collect();
        Self {
            settings_path,
            learned_path,
            replacements_path: data.join("bande/text-replacements"),
            options,
            languages,
            learned,
        }
    }

    pub(crate) fn replacements(&self) -> HashMap<String, String> {
        fs::read_to_string(&self.replacements_path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(from, to)| (from.to_owned(), to.to_owned()))
            .collect()
    }

    pub(crate) fn save_replacements(&self, all: &HashMap<String, String>) -> io::Result<()> {
        let mut contents = String::new();
        for (from, to) in all {
            contents.push_str(&format!("{from}\t{to}\n"));
        }
        bande_xdg::replace_file(&self.replacements_path, &contents)
    }

    pub(crate) fn save_settings(&self) -> io::Result<()> {
        let mut contents = format!("{LANGUAGES_KEY} = {}\n", self.languages.join(" "));
        for (key, value) in &self.options {
            contents.push_str(&format!("{key} = {value}\n"));
        }
        bande_xdg::replace_file(&self.settings_path, &contents)
    }

    pub(crate) fn save_learned(&self) -> io::Result<()> {
        let mut contents = String::new();
        for word in &self.learned {
            contents.push_str(word);
            contents.push('\n');
        }
        bande_xdg::replace_file(&self.learned_path, &contents)
    }
}
