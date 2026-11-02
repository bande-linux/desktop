use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread;

use bande_fs::is_hidden;
use bande_inbox::Outbox;
use bande_volumes::mounted_volumes;
use bande_xdg::{DIRECTORY, FileTypes, config_home};

use crate::model::{Item, Kind, Positions, Slot};

const VOLUME_KIND: &str = "inode/mount-point";
pub(crate) const SETTINGS_FOLDER: &str = "bande";
const FILES_SETTINGS: &str = "files.conf";
const SHOW_EXTENSIONS: &str = "show-extensions";
const SHOW_DISKS: &str = "desktop-disks";
const WARN_EXTENSION_CHANGE: &str = "warn-extension-change";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shown {
    pub extensions: bool,
    pub disks: bool,
    pub warn_extension_change: bool,
}

impl Default for Shown {
    fn default() -> Self {
        Self {
            extensions: false,
            disks: true,
            warn_extension_change: true,
        }
    }
}

impl Shown {
    fn read() -> Self {
        let text = config_home()
            .and_then(|config| {
                fs::read_to_string(config.join(SETTINGS_FOLDER).join(FILES_SETTINGS)).ok()
            })
            .unwrap_or_default();
        let flag = |wanted: &str| {
            text.lines()
                .filter_map(|line| line.split_once('='))
                .find(|(key, _)| key.trim() == wanted)
                .map(|(_, value)| value.trim() == "true")
        };
        Self {
            extensions: flag(SHOW_EXTENSIONS).unwrap_or(false),
            disks: flag(SHOW_DISKS).unwrap_or(true),
            warn_extension_change: flag(WARN_EXTENSION_CHANGE).unwrap_or(true),
        }
    }
}

pub(crate) struct Listing {
    pub shown: Shown,
    pub items: Vec<Item>,
}

pub(crate) fn start_lister(
    mut types: FileTypes,
    desktop: PathBuf,
    media: Vec<PathBuf>,
    listed: Outbox<Listing>,
) -> io::Result<Sender<()>> {
    let (requests, asked) = mpsc::channel::<()>();
    thread::Builder::new()
        .name("desktop lister".to_owned())
        .spawn(move || {
            while asked.recv().is_ok() {
                while asked.try_recv().is_ok() {}
                let shown = Shown::read();
                let volumes = match shown.disks {
                    true => {
                        let mountinfo =
                            fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
                        mounted_volumes(&mountinfo, &media)
                    }
                    false => Vec::new(),
                };
                let items = list(&mut types, &desktop, &volumes, shown.extensions);
                if !listed.send(Listing { shown, items }) {
                    return;
                }
            }
        })?;
    Ok(requests)
}

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn read_item(
    types: &mut FileTypes,
    path: PathBuf,
    kind_hint: Option<Kind>,
    show_extensions: bool,
) -> Item {
    let metadata = fs::metadata(&path).ok();
    let is_dir = metadata.as_ref().is_some_and(fs::Metadata::is_dir);
    let kind = kind_hint.unwrap_or(if is_dir { Kind::Folder } else { Kind::File });
    let mime = match kind {
        Kind::Volume => VOLUME_KIND.to_owned(),
        Kind::Folder => DIRECTORY.to_owned(),
        Kind::File => types.mime_for(&path),
    };
    let name = name_of(&path);
    let hidden = match show_extensions || kind != Kind::File {
        true => 0,
        false => types.extension(&name).map_or(0, str::len),
    };
    Item {
        label: name[..name.len() - hidden].to_owned(),
        name,
        kind_name: types.kind(&mime),
        mime,
        size: metadata.as_ref().map_or(0, fs::Metadata::len),
        added: metadata
            .as_ref()
            .and_then(|metadata| metadata.created().or_else(|_| metadata.modified()).ok()),
        modified: metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok()),
        kind,
        path,
    }
}

fn list(
    types: &mut FileTypes,
    desktop: &Path,
    volumes: &[PathBuf],
    show_extensions: bool,
) -> Vec<Item> {
    let mut items: Vec<Item> = volumes
        .iter()
        .map(|volume| read_item(types, volume.clone(), Some(Kind::Volume), show_extensions))
        .collect();
    if let Ok(entries) = fs::read_dir(desktop) {
        items.extend(
            entries
                .filter_map(Result::ok)
                .filter(|entry| !is_hidden(&entry.file_name().to_string_lossy()))
                .map(|entry| read_item(types, entry.path(), None, show_extensions)),
        );
    }
    items
}

pub type Layouts = HashMap<String, Positions>;

pub fn parse_layouts(text: &str) -> Layouts {
    let mut layouts = Layouts::new();
    for line in text.lines() {
        let mut fields = line.splitn(4, '\t');
        let (Some(grid), Some(column), Some(row), Some(name)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let (Ok(column), Ok(row)) = (column.parse(), row.parse()) else {
            continue;
        };
        layouts
            .entry(grid.to_owned())
            .or_default()
            .insert(name.to_owned(), Slot { column, row });
    }
    layouts
}

pub fn format_layouts(layouts: &Layouts) -> String {
    let mut lines: Vec<String> = layouts
        .iter()
        .flat_map(|(grid, positions)| {
            positions
                .iter()
                .map(move |(name, slot)| format!("{grid}\t{}\t{}\t{name}", slot.column, slot.row))
        })
        .collect();
    lines.sort();
    lines.into_iter().map(|line| line + "\n").collect()
}

pub fn layouts_path() -> Option<PathBuf> {
    bande_xdg::state_home().map(|state| state.join("bande/desktop-layout.tsv"))
}

pub fn save_layouts(path: &Path, layouts: &Layouts) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let partial = path.with_extension("tsv.new");
    fs::write(&partial, format_layouts(layouts))?;
    fs::rename(partial, path)
}
