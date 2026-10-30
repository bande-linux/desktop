use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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
        .join("unbound")
        .join(name)
}

pub fn views_file(state_home: Option<&Path>, home: &Path) -> PathBuf {
    state_file(state_home, home, "files-views")
}

pub fn status_bar_file(state_home: Option<&Path>, home: &Path) -> PathBuf {
    state_file(state_home, home, "files-status-bar")
}

fn parse(text: &str) -> Vec<(PathBuf, FolderView)> {
    let mut seen = HashSet::new();
    let mut folders = Vec::new();
    for line in text.lines() {
        let mut fields = line.splitn(3, '\t');
        let (Some(mode), Some(size), Some(path)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let (Some(mode), Ok(icon_size)) = (ViewMode::parse(mode), size.parse::<u32>()) else {
            continue;
        };
        let view = FolderView {
            mode,
            icon_size: icon_size.clamp(ICON_SIZES.0, ICON_SIZES.1),
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
                    "{}\t{}\t{}\n",
                    view.mode.name(),
                    view.icon_size,
                    path.display()
                )
            })
            .collect::<String>();
        let partial = file.with_extension("partial");
        fs::write(&partial, text)?;
        fs::rename(partial, file)
    }
}
