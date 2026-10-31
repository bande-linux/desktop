use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use bande_fs::{Sort, SortBy};

pub const DEFAULT_ICON_SIZE: u32 = 64;
pub const ICON_SIZES: (u32, u32) = (32, 256);
const KEPT: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    Icons,
    List,
    #[default]
    Columns,
}

impl ViewMode {
    fn name(self) -> &'static str {
        match self {
            Self::Icons => "icons",
            Self::List => "list",
            Self::Columns => "columns",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        [Self::Icons, Self::List, Self::Columns]
            .into_iter()
            .find(|mode| mode.name() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderView {
    pub mode: ViewMode,
    pub icon_size: u32,
    pub sort: Sort,
}

const SORT_NAMES: [(SortBy, &str); 5] = [
    (SortBy::Name, "name"),
    (SortBy::Modified, "modified"),
    (SortBy::Created, "created"),
    (SortBy::Size, "size"),
    (SortBy::Kind, "kind"),
];

fn sort_name(sort: Sort) -> String {
    let by = SORT_NAMES
        .iter()
        .find(|(by, _)| *by == sort.by)
        .map_or("name", |(_, name)| name);
    match sort.descending {
        true => format!("{by}-descending"),
        false => by.to_owned(),
    }
}

fn parse_sort(text: &str) -> Option<Sort> {
    let (name, descending) = match text.strip_suffix("-descending") {
        Some(name) => (name, true),
        None => (text, false),
    };
    SORT_NAMES
        .iter()
        .find(|(_, known)| *known == name)
        .map(|(by, _)| Sort {
            by: *by,
            descending,
        })
}

#[derive(Debug, Clone, Default)]
pub struct ViewMemory {
    file: Option<PathBuf>,
    folders: Vec<(PathBuf, FolderView)>,
}

fn state_file(state_home: Option<&Path>, home: &Path, name: &str) -> PathBuf {
    state_home
        .filter(|path| path.is_absolute())
        .map_or_else(|| home.join(".local/state"), Path::to_path_buf)
        .join("bande")
        .join(name)
}

pub fn views_file(state_home: Option<&Path>, home: &Path) -> PathBuf {
    state_file(state_home, home, "files-views")
}

pub fn sidebar_file(state_home: Option<&Path>, home: &Path) -> PathBuf {
    state_file(state_home, home, "files-sidebar")
}

pub fn list_columns_file(state_home: Option<&Path>, home: &Path) -> PathBuf {
    state_file(state_home, home, "files-columns")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListColumn {
    Modified,
    Created,
    Size,
    Kind,
}

impl ListColumn {
    pub const ALL: [Self; 4] = [Self::Modified, Self::Created, Self::Size, Self::Kind];
    pub const DEFAULT: [Self; 3] = [Self::Modified, Self::Size, Self::Kind];

    pub fn name(self) -> &'static str {
        match self {
            Self::Modified => "modified",
            Self::Created => "created",
            Self::Size => "size",
            Self::Kind => "kind",
        }
    }

    pub fn sort(self) -> SortBy {
        match self {
            Self::Modified => SortBy::Modified,
            Self::Created => SortBy::Created,
            Self::Size => SortBy::Size,
            Self::Kind => SortBy::Kind,
        }
    }
}

pub fn load_list_columns(file: &Path) -> Vec<ListColumn> {
    let Ok(text) = fs::read_to_string(file) else {
        return ListColumn::DEFAULT.to_vec();
    };
    ListColumn::ALL
        .into_iter()
        .filter(|column| text.lines().any(|line| line.trim() == column.name()))
        .collect()
}

pub fn save_list_columns(file: &Path, columns: &[ListColumn]) -> io::Result<()> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = columns
        .iter()
        .map(|column| format!("{}\n", column.name()))
        .collect::<String>();
    let partial = file.with_extension("partial");
    fs::write(&partial, text)?;
    fs::rename(partial, file)
}

pub fn status_bar_file(state_home: Option<&Path>, home: &Path) -> PathBuf {
    state_file(state_home, home, "files-status-bar")
}

fn parse(text: &str) -> Vec<(PathBuf, FolderView)> {
    let mut seen = HashSet::new();
    let mut folders = Vec::new();
    for line in text.lines() {
        let mut fields = line.splitn(4, '\t');
        let (Some(mode), Some(size), Some(sort), Some(path)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let (Some(mode), Ok(icon_size), Some(sort)) =
            (ViewMode::parse(mode), size.parse::<u32>(), parse_sort(sort))
        else {
            continue;
        };
        let view = FolderView {
            mode,
            icon_size: icon_size.clamp(ICON_SIZES.0, ICON_SIZES.1),
            sort,
        };
        if seen.insert(PathBuf::from(path)) {
            folders.push((PathBuf::from(path), view));
        }
    }
    folders
}

impl ViewMemory {
    pub fn load(file: PathBuf) -> Self {
        let folders = fs::read_to_string(&file)
            .map(|text| parse(&text))
            .unwrap_or_default();
        Self {
            file: Some(file),
            folders,
        }
    }

    pub fn get(&self, folder: &Path) -> Option<FolderView> {
        self.folders
            .iter()
            .find(|(known, _)| known == folder)
            .map(|(_, view)| *view)
    }

    pub fn remember(&mut self, folder: &Path, view: FolderView) -> io::Result<()> {
        if self.get(folder) == Some(view) {
            return Ok(());
        }
        self.folders.retain(|(known, _)| known != folder);
        self.folders.insert(0, (folder.to_path_buf(), view));
        self.folders.truncate(KEPT);
        let Some(file) = &self.file else {
            return Ok(());
        };
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = self
            .folders
            .iter()
            .filter(|(path, _)| !path.to_string_lossy().contains('\n'))
            .map(|(path, view)| {
                format!(
                    "{}\t{}\t{}\t{}\n",
                    view.mode.name(),
                    view.icon_size,
                    sort_name(view.sort),
                    path.display()
                )
            })
            .collect::<String>();
        let partial = file.with_extension("partial");
        fs::write(&partial, text)?;
        fs::rename(partial, file)
    }
}
