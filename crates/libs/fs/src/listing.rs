use std::cmp::Ordering;
use std::fs;
use std::io;
use std::iter::Peekable;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::str::Chars;
use std::time::SystemTime;

use crate::trash::{Trash, TrashItem};

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
    pub created: Option<SystemTime>,
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
            modified: metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok()),
            created: metadata.and_then(|metadata| metadata.created().ok()),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortBy {
    #[default]
    Name,
    Modified,
    Size,
    Kind,
    Created,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sort {
    pub by: SortBy,
    pub descending: bool,
}

pub fn compare(a: &Entry, b: &Entry, sort: Sort) -> Ordering {
    let order = match sort.by {
        SortBy::Name => finder_order(&a.name, &b.name),
        SortBy::Modified => b.modified.cmp(&a.modified),
        SortBy::Size => b.size.cmp(&a.size),
        SortBy::Kind => kind_key(a)
            .chars()
            .flat_map(char::to_lowercase)
            .cmp(kind_key(b).chars().flat_map(char::to_lowercase)),
        SortBy::Created => b.created.cmp(&a.created),
    }
    .then_with(|| finder_order(&a.name, &b.name));
    b.is_folder().cmp(&a.is_folder()).then(if sort.descending {
        order.reverse()
    } else {
        order
    })
}

fn kind_key(entry: &Entry) -> &str {
    match entry.name.rsplit_once('.') {
        Some((stem, extension)) if !entry.is_folder() && !stem.is_empty() => extension,
        _ => "",
    }
}

pub fn sort(entries: &mut [Entry], order: Sort) {
    entries.sort_by(|a, b| compare(a, b, order));
}

pub fn merge(entries: &mut Vec<Entry>, chunk: Vec<Entry>, order: Sort) -> Vec<usize> {
    let shown = std::mem::take(entries);
    let mut moved = Vec::with_capacity(shown.len());
    let mut merged = Vec::with_capacity(shown.len() + chunk.len());
    let mut shown = shown.into_iter().peekable();
    let mut chunk = chunk.into_iter().peekable();
    loop {
        let take_shown = match (shown.peek(), chunk.peek()) {
            (Some(old), Some(new)) => compare(old, new, order) != Ordering::Greater,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        let next = match take_shown {
            true => {
                moved.push(merged.len());
                shown.next()
            }
            false => chunk.next(),
        };
        merged.extend(next);
    }
    *entries = merged;
    moved
}

#[derive(Debug)]
pub struct Listing {
    folder: PathBuf,
    names: fs::ReadDir,
    show_hidden: bool,
    done: bool,
}

impl Listing {
    pub fn open(folder: &Path, show_hidden: bool) -> io::Result<Self> {
        Ok(Self {
            folder: folder.to_path_buf(),
            names: fs::read_dir(folder)?,
            show_hidden,
            done: false,
        })
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    pub fn next_chunk(&mut self, count: usize) -> Vec<Entry> {
        let mut entries = Vec::new();
        while entries.len() < count {
            let Some(found) = self.names.next() else {
                self.done = true;
                break;
            };
            let Ok(found) = found else {
                continue;
            };
            let name = found.file_name().to_string_lossy().into_owned();
            if self.show_hidden || !is_hidden(&name) {
                entries.push(Entry::read(self.folder.join(&name), name));
            }
        }
        entries
    }
}

pub fn list(folder: &Path, show_hidden: bool, order: Sort) -> io::Result<Vec<Entry>> {
    let mut entries = Listing::open(folder, show_hidden)?.next_chunk(usize::MAX);
    sort(&mut entries, order);
    Ok(entries)
}

pub fn list_trash(trash: &Trash, order: Sort) -> Vec<Entry> {
    let mut entries = trash
        .items()
        .into_iter()
        .map(|item| {
            let mut entry = Entry::read(item.path(), item.name());
            entry.trashed = Some(item);
            entry
        })
        .collect::<Vec<_>>();
    sort(&mut entries, order);
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

pub fn same_volume(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev(),
        _ => false,
    }
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

pub fn format_list_date(time: SystemTime) -> String {
    list_date(time, SystemTime::now(), &bande_datetime::local_zone())
}

pub fn list_date(time: SystemTime, now: SystemTime, zone: &jiff::tz::TimeZone) -> String {
    let (Ok(time), Ok(now)) = (
        jiff::Timestamp::try_from(time),
        jiff::Timestamp::try_from(now),
    ) else {
        return String::new();
    };
    let (time, now) = (time.to_zoned(zone.clone()), now.to_zoned(zone.clone()));
    let clock = time.strftime("%H:%M").to_string();
    let day = time.date();
    if day == now.date() {
        L10N.tr_args("date-today", [("time", clock.into())])
    } else if now
        .date()
        .yesterday()
        .is_ok_and(|yesterday| yesterday == day)
    {
        L10N.tr_args("date-yesterday", [("time", clock.into())])
    } else if day.year() == now.date().year() {
        time.strftime("%-d %b, %H:%M").to_string()
    } else {
        time.strftime("%-d %b %Y, %H:%M").to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recency {
    AnyTime,
    Today,
    LastWeek,
    LastMonth,
    ThisYear,
}

impl Recency {
    pub const ALL: [Self; 5] = [
        Self::AnyTime,
        Self::Today,
        Self::LastWeek,
        Self::LastMonth,
        Self::ThisYear,
    ];

    pub fn start(self, now: SystemTime, zone: &jiff::tz::TimeZone) -> Option<SystemTime> {
        let now = jiff::Timestamp::try_from(now).ok()?.to_zoned(zone.clone());
        let today = now.start_of_day().ok()?;
        let start = match self {
            Self::AnyTime => return None,
            Self::Today => today,
            Self::LastWeek => today.checked_sub(jiff::Span::new().days(6)).ok()?,
            Self::LastMonth => today.checked_sub(jiff::Span::new().days(29)).ok()?,
            Self::ThisYear => today.first_of_year().ok()?,
        };
        Some(SystemTime::from(start.timestamp()))
    }

    pub fn includes(self, time: Option<SystemTime>) -> bool {
        match self.start(SystemTime::now(), &bande_datetime::local_zone()) {
            None => true,
            Some(start) => time.is_some_and(|time| time >= start),
        }
    }
}

pub fn format_date(time: SystemTime) -> String {
    jiff::Timestamp::try_from(time)
        .map(|timestamp| {
            timestamp
                .to_zoned(bande_datetime::local_zone())
                .strftime("%-d %b %Y, %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}
