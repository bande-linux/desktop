use std::cmp::Ordering;
use std::fs;
use std::io;
use std::iter::Peekable;
use std::path::{Path, PathBuf};
use std::str::Chars;
use std::time::SystemTime;

use unbound_fs::{Trash, TrashItem};

use crate::L10N;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Folder,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub link: bool,
    pub trashed: Option<TrashItem>,
}

impl Entry {
    pub fn read(path: PathBuf, name: String) -> Self {
        let link = fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_symlink());
        let metadata = fs::metadata(&path).ok();
        Self {
            name,
            kind: if metadata.as_ref().is_some_and(fs::Metadata::is_dir) {
                Kind::Folder
            } else {
                Kind::File
            },
            size: metadata.as_ref().map_or(0, fs::Metadata::len),
            modified: metadata.and_then(|metadata| metadata.modified().ok()),
            link,
            path,
            trashed: None,
        }
    }

    pub fn is_folder(&self) -> bool {
        self.kind == Kind::Folder
    }
}

pub fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

pub fn sort(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        b.is_folder()
            .cmp(&a.is_folder())
            .then_with(|| finder_order(&a.name, &b.name))
    });
}

pub fn list(folder: &Path, show_hidden: bool) -> io::Result<Vec<Entry>> {
    let mut entries = fs::read_dir(folder)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| show_hidden || !is_hidden(name))
        .map(|name| Entry::read(folder.join(&name), name))
        .collect::<Vec<_>>();
    sort(&mut entries);
    Ok(entries)
}

pub fn list_trash(trash: &Trash) -> Vec<Entry> {
    let mut entries = trash
        .items()
        .into_iter()
        .map(|item| {
            let mut entry = Entry::read(item.path(), item.name());
            entry.trashed = Some(item);
            entry
        })
        .collect::<Vec<_>>();
    sort(&mut entries);
    entries
}

fn digits(chars: &mut Peekable<Chars<'_>>) -> String {
    let mut run = String::new();
    while let Some(digit) = chars.next_if(char::is_ascii_digit) {
        run.push(digit);
    }
    run
}

fn compare_numbers(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.trim_start_matches('0'), b.trim_start_matches('0'));
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

pub fn finder_order(a: &str, b: &str) -> Ordering {
    let (mut left, mut right) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(l), Some(r)) if l.is_ascii_digit() && r.is_ascii_digit() => {
                let order = compare_numbers(&digits(&mut left), &digits(&mut right));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(l), Some(r)) => {
                let order = l.to_lowercase().cmp(r.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                left.next();
                right.next();
            }
        }
    }
}

pub fn format_size(bytes: u64) -> String {
    if bytes < 1_000 {
        return L10N.tr_args("size-bytes", [("count", bytes.into())]);
    }
    let kilobytes = (bytes as f64 / 1e3).round();
    if kilobytes < 1e3 {
        return L10N.tr_args("size-kilobytes", [("value", kilobytes.to_string().into())]);
    }
    let units = [(1e6, 1), (1e9, 2), (1e12, 2)];
    let (unit, (scale, decimals)) = units
        .into_iter()
        .enumerate()
        .find(|(_, (scale, decimals))| {
            (bytes as f64 / scale * 10f64.powi(*decimals)).round() < 1e3 * 10f64.powi(*decimals)
        })
        .unwrap_or((2, units[2]));
    let value = format!("{:.*}", decimals as usize, bytes as f64 / scale);
    let value = value.trim_end_matches('0').trim_end_matches('.').to_owned();
    let args = [("value", value.into())];
    match unit {
        0 => L10N.tr_args("size-megabytes", args),
        1 => L10N.tr_args("size-gigabytes", args),
        _ => L10N.tr_args("size-terabytes", args),
    }
}
