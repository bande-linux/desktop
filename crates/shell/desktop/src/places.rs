use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use bande_fs::{extension_hidden, is_hidden};
use bande_xdg::{Apps, DIRECTORY};

use crate::model::{Item, Kind, Positions, Slot};

const VOLUME_KIND: &str = "inode/mount-point";

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

pub fn read_item(
    apps: &mut Apps,
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
        Kind::File => apps.mime_for(&path),
    };
    let name = name_of(&path);
    let hidden = match show_extensions || kind != Kind::File {
        true => 0,
        false => bande_fs::hidden_extension(&name, apps.extension(&name), extension_hidden(&path))
            .map_or(0, str::len),
    };
    Item {
        label: name[..name.len() - hidden].to_owned(),
        name,
        kind_name: apps.kind(&mime),
        size: metadata.as_ref().map_or(0, fs::Metadata::len),
        added: metadata
            .as_ref()
            .and_then(|metadata| metadata.created().or_else(|_| metadata.modified()).ok()),
        kind,
        path,
    }
}

pub fn list(
    apps: &mut Apps,
    desktop: &Path,
    volumes: &[PathBuf],
    show_extensions: bool,
) -> Vec<Item> {
    let mut items: Vec<Item> = volumes
        .iter()
        .map(|volume| read_item(apps, volume.clone(), Some(Kind::Volume), show_extensions))
        .collect();
    if let Ok(entries) = fs::read_dir(desktop) {
        items.extend(
            entries
                .filter_map(Result::ok)
                .filter(|entry| !is_hidden(&entry.file_name().to_string_lossy()))
                .map(|entry| read_item(apps, entry.path(), None, show_extensions)),
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
