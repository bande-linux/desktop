use std::cmp::Ordering;
use std::fs;
use std::io;
use std::iter::Peekable;
use std::path::{Path, PathBuf};
use std::str::Chars;

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
}

pub fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

pub fn list(folder: &Path, show_hidden: bool) -> io::Result<Vec<Entry>> {
    let mut entries = fs::read_dir(folder)?
        .filter_map(Result::ok)
        .map(|entry| {
            let path = entry.path();
            Entry {
                name: entry.file_name().to_string_lossy().into_owned(),
                kind: if path.is_dir() {
                    Kind::Folder
                } else {
                    Kind::File
                },
                path,
            }
        })
        .filter(|entry| show_hidden || !is_hidden(&entry.name))
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| finder_order(&a.name, &b.name));
    Ok(entries)
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
    match bytes {
        0 => return "Zero bytes".to_owned(),
        1 => return "1 byte".to_owned(),
        2..1_000 => return format!("{bytes} bytes"),
        _ => {}
    }
    let kilobytes = (bytes as f64 / 1e3).round();
    if kilobytes < 1e3 {
        return format!("{kilobytes} KB");
    }
    let units = [(1e6, "MB", 1), (1e9, "GB", 2), (1e12, "TB", 2)];
    let (scale, unit, decimals) = units
        .into_iter()
        .find(|(scale, _, decimals)| {
            (bytes as f64 / scale * 10f64.powi(*decimals)).round() < 1e3 * 10f64.powi(*decimals)
        })
        .unwrap_or(units[2]);
    let value = format!("{:.*}", decimals as usize, bytes as f64 / scale);
    let value = value.trim_end_matches('0').trim_end_matches('.');
    format!("{value} {unit}")
}
