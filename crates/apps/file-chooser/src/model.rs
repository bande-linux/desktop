use std::io;
use std::path::{Path, PathBuf};

use bande_fs::{Entry, Sort, list, sort};
use bande_xdg::{MimeDatabase, file_uri, path_from_file_uri};
use xdg_desktop_portal_bande::file_chooser::{Dialog, Filter, Mode, Pattern, Selection};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accept {
    Done(Selection),
    Enter(PathBuf),
    Replace(PathBuf),
    Nothing,
}

fn class_matches(class: &[char], wanted: char) -> Option<(bool, usize)> {
    let negated = matches!(class.first(), Some('!' | '^'));
    let mut index = usize::from(negated);
    let mut found = false;
    let mut first = true;
    while let Some(&current) = class.get(index) {
        if current == ']' && !first {
            return Some((found != negated, index + 1));
        }
        first = false;
        if class.get(index + 1) == Some(&'-')
            && let Some(&last) = class.get(index + 2).filter(|last| **last != ']')
        {
            found |= (current..=last).contains(&wanted);
            index += 3;
        } else {
            found |= current == wanted;
            index += 1;
        }
    }
    None
}

fn glob(pattern: &[char], name: &[char]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some(('*', rest)) => (0..=name.len()).any(|skip| glob(rest, &name[skip..])),
        Some(('?', rest)) => name.split_first().is_some_and(|(_, name)| glob(rest, name)),
        Some(('[', class)) => {
            name.split_first()
                .is_some_and(|(&wanted, name)| match class_matches(class, wanted) {
                    Some((true, used)) => glob(&class[used..], name),
                    Some((false, _)) => false,
                    None => wanted == '[' && glob(class, name),
                })
        }
        Some((&literal, rest)) => name
            .split_first()
            .is_some_and(|(&first, name)| first == literal && glob(rest, name)),
    }
}

pub fn glob_matches(pattern: &str, name: &str) -> bool {
    let lower = |text: &str| text.to_lowercase().chars().collect::<Vec<_>>();
    glob(&lower(pattern), &lower(name))
}

pub fn remembered_folder(memory: &str, app_id: &str) -> Option<PathBuf> {
    memory
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .find(|(app, _)| *app == app_id)
        .and_then(|(_, folder)| path_from_file_uri(folder))
}

pub fn remember(memory: &str, app_id: &str, folder: &Path) -> String {
    let mut lines: Vec<String> = memory
        .lines()
        .filter(|line| line.split_once('\t').is_none_or(|(app, _)| app != app_id))
        .map(str::to_owned)
        .collect();
    lines.push(format!("{app_id}\t{}", file_uri(folder)));
    lines.join("\n") + "\n"
}

pub fn initial_folder(
    dialog: &Dialog,
    remembered: Option<PathBuf>,
    home: &Path,
    documents: &Path,
) -> PathBuf {
    let given = match &dialog.mode {
        Mode::Save {
            file: Some(file), ..
        } => file.parent().map(Path::to_path_buf),
        _ => None,
    };
    let fallback = match dialog.mode {
        Mode::Save { .. } | Mode::SaveMany { .. } => documents,
        Mode::Open { .. } => home,
    };
    [dialog.folder.clone(), given, remembered]
        .into_iter()
        .flatten()
        .find(|folder| folder.is_dir())
        .unwrap_or_else(|| fallback.to_path_buf())
}

pub fn initial_name(dialog: &Dialog) -> String {
    match &dialog.mode {
        Mode::Save {
            name: Some(name), ..
        } => name.clone(),
        Mode::Save {
            file: Some(file), ..
        } => file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

pub struct Chooser {
    dialog: Dialog,
    mime: MimeDatabase,
    folder: PathBuf,
    entries: Vec<Entry>,
    filter: Option<usize>,
    sort: Sort,
    pub name: String,
}

impl Chooser {
    pub fn new(dialog: Dialog, mime: MimeDatabase, folder: PathBuf) -> Self {
        let filter = dialog
            .current_filter
            .as_ref()
            .and_then(|current| dialog.filters.iter().position(|filter| filter == current))
            .or_else(|| (!dialog.filters.is_empty()).then_some(0));
        let mut chooser = Self {
            name: initial_name(&dialog),
            dialog,
            mime,
            folder: PathBuf::new(),
            entries: Vec::new(),
            filter,
            sort: Sort::default(),
        };
        if chooser.open(&folder).is_err() {
            chooser.folder = folder;
        }
        chooser
    }

    pub fn dialog(&self) -> &Dialog {
        &self.dialog
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn filter(&self) -> Option<usize> {
        self.filter
    }

    pub fn filters(&self) -> &[Filter] {
        &self.dialog.filters
    }

    pub fn open(&mut self, folder: &Path) -> io::Result<()> {
        self.entries = list(folder, false, self.sort)?;
        self.folder = folder.to_path_buf();
        Ok(())
    }

    pub fn set_filter(&mut self, filter: usize) {
        if filter < self.dialog.filters.len() {
            self.filter = Some(filter);
        }
    }

    pub fn set_sort(&mut self, order: Sort) {
        self.sort = order;
        sort(&mut self.entries, order);
    }

    pub fn sort(&self) -> Sort {
        self.sort
    }

    fn chooses_folders(&self) -> bool {
        matches!(
            self.dialog.mode,
            Mode::Open {
                directory: true,
                ..
            } | Mode::SaveMany { .. }
        )
    }

    fn shows(&self, entry: &Entry) -> bool {
        if entry.is_folder() {
            return true;
        }
        if self.chooses_folders() {
            return false;
        }
        let Some(filter) = self.filter.and_then(|index| self.dialog.filters.get(index)) else {
            return true;
        };
        let mime = self.mime.for_name(&entry.name);
        filter.patterns.iter().any(|pattern| match pattern {
            Pattern::Glob(pattern) => glob_matches(pattern, &entry.name),
            Pattern::MimeType(wanted) => mime.is_some_and(|mime| {
                wanted == "*/*"
                    || wanted
                        .strip_suffix("/*")
                        .is_some_and(|family| mime.split('/').next() == Some(family))
                    || self.mime.is_a(mime, wanted)
            }),
        })
    }

    pub fn entries(&self) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|entry| self.shows(entry))
            .collect()
    }

    pub fn mime(&self) -> &MimeDatabase {
        &self.mime
    }

    fn selection(&self, paths: Vec<PathBuf>) -> Selection {
        Selection {
            paths,
            choices: Vec::new(),
            filter: self
                .filter
                .and_then(|index| self.dialog.filters.get(index))
                .cloned(),
            writable: true,
        }
    }

    pub fn accept(&self, selected: &[PathBuf]) -> Accept {
        let is_folder = |path: &PathBuf| path.is_dir();
        match &self.dialog.mode {
            Mode::Open {
                directory: false,
                multiple,
            } => {
                if let [only] = selected
                    && is_folder(only)
                {
                    return Accept::Enter(only.clone());
                }
                let files: Vec<PathBuf> = selected
                    .iter()
                    .filter(|path| !is_folder(path))
                    .take(if *multiple { usize::MAX } else { 1 })
                    .cloned()
                    .collect();
                match files.is_empty() {
                    true => Accept::Nothing,
                    false => Accept::Done(self.selection(files)),
                }
            }
            Mode::Open {
                directory: true,
                multiple,
            } => {
                let folders: Vec<PathBuf> = selected
                    .iter()
                    .filter(|path| is_folder(path))
                    .take(if *multiple { usize::MAX } else { 1 })
                    .cloned()
                    .collect();
                match folders.is_empty() {
                    true => Accept::Done(self.selection(vec![self.folder.clone()])),
                    false => Accept::Done(self.selection(folders)),
                }
            }
            Mode::SaveMany { .. } => match selected {
                [only] if is_folder(only) => Accept::Done(self.selection(vec![only.clone()])),
                _ => Accept::Done(self.selection(vec![self.folder.clone()])),
            },
            Mode::Save { .. } => {
                let name = self.name.trim();
                if name.is_empty() || name.contains('/') || name == "." || name == ".." {
                    return Accept::Nothing;
                }
                let path = self.folder.join(name);
                if path.is_dir() {
                    Accept::Enter(path)
                } else if path.exists() {
                    Accept::Replace(path)
                } else {
                    Accept::Done(self.selection(vec![path]))
                }
            }
        }
    }

    pub fn replace(&self, path: PathBuf) -> Selection {
        self.selection(vec![path])
    }
}
