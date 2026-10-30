use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use unbound_files::{Apps, is_hidden};
use unbound_xdg::DIRECTORY;

use crate::model::{Item, Kind, Positions, Slot};

const VOLUME_KIND: &str = "inode/mount-point";

pub fn media_roots(user: &str) -> Vec<PathBuf> {
    vec![
        Path::new("/run/media").join(user),
        Path::new("/media").join(user),
    ]
}

fn unescape(field: &str) -> String {
    let mut out = Vec::with_capacity(field.len());
    let bytes = field.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let octal = bytes
            .get(index + 1..index + 4)
            .filter(|_| bytes[index] == b'\\')
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u8::from_str_radix(digits, 8).ok());
        match octal {
            Some(byte) => {
                out.push(byte);
                index += 4;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn mounted_volumes(mountinfo: &str, roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut volumes: Vec<PathBuf> = mountinfo
        .lines()
        .filter_map(|line| line.split(' ').nth(4))
        .map(|field| PathBuf::from(unescape(field)))
        .filter(|point| {
            roots
                .iter()
                .any(|root| point.parent() == Some(root.as_path()))
        })
        .collect();
    volumes.sort();
    volumes.dedup();
    volumes
}

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

pub fn read_item(apps: &mut Apps, path: PathBuf, kind_hint: Option<Kind>) -> Item {
    let metadata = fs::metadata(&path).ok();
    let is_dir = metadata.as_ref().is_some_and(fs::Metadata::is_dir);
    let kind = kind_hint.unwrap_or(if is_dir { Kind::Folder } else { Kind::File });
    let mime = match kind {
        Kind::Volume => VOLUME_KIND.to_owned(),
        Kind::Folder => DIRECTORY.to_owned(),
        Kind::File => apps.mime_for(&path),
    };
    Item {
        name: name_of(&path),
        kind_name: apps.kind(&mime),
        size: metadata.as_ref().map_or(0, fs::Metadata::len),
        added: metadata
            .as_ref()
            .and_then(|metadata| metadata.created().or_else(|_| metadata.modified()).ok()),
        kind,
        path,
    }
}

pub fn list(apps: &mut Apps, desktop: &Path, volumes: &[PathBuf]) -> Vec<Item> {
    let mut items: Vec<Item> = volumes
        .iter()
        .map(|volume| read_item(apps, volume.clone(), Some(Kind::Volume)))
        .collect();
    if let Ok(entries) = fs::read_dir(desktop) {
        items.extend(
            entries
                .filter_map(Result::ok)
                .filter(|entry| !is_hidden(&entry.file_name().to_string_lossy()))
                .map(|entry| read_item(apps, entry.path(), None)),
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
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/state")))
        .map(|state| state.join("unbound/desktop-layout.tsv"))
}

pub fn save_layouts(path: &Path, layouts: &Layouts) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let partial = path.with_extension("tsv.new");
    fs::write(&partial, format_layouts(layouts))?;
    fs::rename(partial, path)
}
