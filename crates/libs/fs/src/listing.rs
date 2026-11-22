use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

use bande_l10n::{tr, trn};
use icu_collator::options::CollatorOptions;
use icu_collator::preferences::CollationNumericOrdering;
use icu_collator::{CollatorBorrowed, CollatorPreferences};
use icu_decimal::input::Decimal;
use icu_decimal::{DecimalFormatter, DecimalFormatterPreferences};

use crate::group::{GroupBy, group_of};
use crate::partial::{is_partial, sweep};

use crate::trash::{Trash, TrashItem};

use crate::DATES;

static NAMES: LazyLock<Option<CollatorBorrowed<'static>>> = LazyLock::new(|| {
    let mut preferences = CollatorPreferences::from(&bande_datetime::env_locale(&[
        "LC_ALL",
        "LC_COLLATE",
        "LANG",
    ]));
    preferences.numeric_ordering = Some(CollationNumericOrdering::True);
    CollatorBorrowed::try_new(preferences, CollatorOptions::default())
        .inspect_err(|error| tracing::warn!("cannot sort names for this language: {error}"))
        .ok()
});

thread_local! {
    static NUMBERS: Option<DecimalFormatter> = {
        let locale = bande_datetime::env_locale(&["LC_ALL", "LC_NUMERIC", "LANG"]);
        DecimalFormatter::try_new(DecimalFormatterPreferences::from(&locale), Default::default())
            .inspect_err(|error| tracing::warn!("cannot write numbers for this language: {error}"))
            .ok()
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Folder,
    File,
    Server,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub added: Option<SystemTime>,
    pub link: bool,
    pub trashed: Option<TrashItem>,
    pub extension_hidden: Option<bool>,
    pub mode: Option<u32>,
    pub owned: bool,
}

impl Entry {
    pub fn read(path: PathBuf, name: String) -> Self {
        let link = fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_symlink());
        if link && bande_volumes::leads_to_connected_volume(&path) {
            return Self::link_to_connected(path, name);
        }
        let metadata = fs::metadata(&path).ok();
        let mode = metadata.as_ref().map(MetadataExt::mode);
        let owned = metadata
            .as_ref()
            .is_some_and(|metadata| metadata.uid() == rustix::process::getuid().as_raw());
        let kind = match metadata.as_ref().is_some_and(fs::Metadata::is_dir) {
            true => Kind::Folder,
            false => Kind::File,
        };
        Self {
            name,
            kind,
            size: metadata.as_ref().map_or(0, fs::Metadata::len),
            modified: metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok()),
            created: metadata
                .as_ref()
                .and_then(|metadata| metadata.created().ok()),
            added: metadata.and_then(|metadata| {
                let seconds = u64::try_from(metadata.ctime()).ok()?;
                let nanoseconds = u32::try_from(metadata.ctime_nsec()).ok()?;
                SystemTime::UNIX_EPOCH.checked_add(Duration::new(seconds, nanoseconds))
            }),
            link,
            extension_hidden: match link || kind == Kind::Folder {
                true => None,
                false => crate::extension_hidden(&path),
            },
            path,
            trashed: None,
            mode,
            owned,
        }
    }

    fn link_to_connected(path: PathBuf, name: String) -> Self {
        Self {
            kind: match Path::new(&name).extension() {
                Some(_) => Kind::File,
                None => Kind::Folder,
            },
            name,
            path,
            size: 0,
            modified: None,
            created: None,
            added: None,
            link: true,
            trashed: None,
            extension_hidden: None,
            mode: None,
            owned: false,
        }
    }

    pub fn server(name: String, address: String) -> Self {
        Self {
            name,
            path: PathBuf::from(address),
            kind: Kind::Server,
            size: 0,
            modified: None,
            created: None,
            added: None,
            link: false,
            trashed: None,
            extension_hidden: None,
            mode: None,
            owned: false,
        }
    }

    pub fn is_folder(&self) -> bool {
        self.kind == Kind::Folder
    }

    pub fn server_address(&self) -> Option<&str> {
        match self.kind {
            Kind::Server => self.path.to_str(),
            Kind::Folder | Kind::File => None,
        }
    }
}

pub fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

pub const HIDDEN_LIST: &str = ".hidden";
const LARGEST_HIDDEN_LIST: u64 = 64 << 10;

/// The names a folder's `.hidden` file lists, one per line, to leave out like dot files.
pub fn hidden_names(folder: &Path) -> HashSet<String> {
    let mut text = String::new();
    let read = fs::File::open(folder.join(HIDDEN_LIST)).and_then(|file| {
        io::Read::read_to_string(&mut io::Read::take(file, LARGEST_HIDDEN_LIST), &mut text)
    });
    match read {
        Ok(_) => text
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect(),
        Err(error) => {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::debug!(
                    "cannot read {}: {error}",
                    folder.join(HIDDEN_LIST).display()
                );
            }
            HashSet::new()
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub by: SortBy,
    pub descending: bool,
    pub folders_first: bool,
    pub group: Option<GroupBy>,
}

impl Default for Sort {
    fn default() -> Self {
        Self {
            by: SortBy::default(),
            descending: false,
            folders_first: true,
            group: None,
        }
    }
}

pub fn compare(a: &Entry, b: &Entry, sort: Sort) -> Ordering {
    let order = match sort.by {
        SortBy::Name => natural_order(&a.name, &b.name),
        SortBy::Modified => b.modified.cmp(&a.modified),
        SortBy::Size => b.size.cmp(&a.size),
        SortBy::Kind => kind_key(a)
            .chars()
            .flat_map(char::to_lowercase)
            .cmp(kind_key(b).chars().flat_map(char::to_lowercase)),
        SortBy::Created => b.created.cmp(&a.created),
    }
    .then_with(|| natural_order(&a.name, &b.name));
    let folders = match sort.folders_first {
        true => b.is_folder().cmp(&a.is_folder()),
        false => Ordering::Equal,
    };
    let grouped = sort
        .group
        .map_or(Ordering::Equal, |by| group_of(a, by).cmp(&group_of(b, by)));
    grouped.then(folders).then(if sort.descending {
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
    names: fs::ReadDir,
    show_hidden: bool,
    listed_hidden: HashSet<String>,
    done: bool,
    failure: Option<io::ErrorKind>,
}

impl Listing {
    pub fn open(folder: &Path, show_hidden: bool) -> io::Result<Self> {
        Ok(Self {
            names: fs::read_dir(folder)?,
            show_hidden,
            listed_hidden: match show_hidden {
                true => HashSet::new(),
                false => hidden_names(folder),
            },
            done: false,
            failure: None,
        })
    }

    pub fn failure(&self) -> Option<io::ErrorKind> {
        self.failure
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
            let found = match found {
                Ok(found) => found,
                Err(error) => {
                    self.failure = Some(error.kind());
                    self.done = true;
                    break;
                }
            };
            let name = found.file_name().to_string_lossy().into_owned();
            if is_partial(&name) {
                sweep(found.path());
            } else if self.show_hidden || !(is_hidden(&name) || self.listed_hidden.contains(&name))
            {
                entries.push(Entry::read(found.path(), name));
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

pub fn same_volume(a: &Path, b: &Path) -> bool {
    let (on_a, on_b) = (
        bande_volumes::connected_volume_of(a),
        bande_volumes::connected_volume_of(b),
    );
    if on_a.is_some() || on_b.is_some() {
        return on_a == on_b;
    }
    match (fs::symlink_metadata(a), fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev(),
        _ => false,
    }
}

pub fn natural_order(a: &str, b: &str) -> Ordering {
    match &*NAMES {
        Some(collator) => collator.compare(a, b),
        None => a.to_lowercase().cmp(&b.to_lowercase()),
    }
    .then_with(|| a.cmp(b))
}

fn decimal(value: f64, decimals: i32) -> String {
    let mut number = Decimal::from((value * 10f64.powi(decimals)).round() as i64);
    number.multiply_pow10(-(decimals as i16));
    number.trim_end();
    NUMBERS.with(|formatter| match formatter {
        Some(formatter) => formatter.format(&number).to_string(),
        None => number.to_string(),
    })
}

pub fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        return tr!("Zero bytes");
    }
    if bytes < 1_000 {
        return trn!("{count} byte", "{count} bytes", count = bytes);
    }
    let kilobytes = (bytes as f64 / 1e3).round();
    if kilobytes < 1e3 {
        return tr!("{value} KB", value = decimal(kilobytes, 0));
    }
    let units = [(1e6, 1), (1e9, 2), (1e12, 2)];
    let (unit, (scale, decimals)) = units
        .into_iter()
        .enumerate()
        .find(|(_, (scale, decimals))| {
            (bytes as f64 / scale * 10f64.powi(*decimals)).round() < 1e3 * 10f64.powi(*decimals)
        })
        .unwrap_or((2, units[2]));
    let value = decimal(bytes as f64 / scale, decimals);
    match unit {
        0 => tr!("{value} MB", value = value),
        1 => tr!("{value} GB", value = value),
        _ => tr!("{value} TB", value = value),
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
    let clock = DATES.clock_time(&time);
    let day = time.date();
    if day == now.date() {
        tr!("Today, {time}", time = clock)
    } else if now
        .date()
        .yesterday()
        .is_ok_and(|yesterday| yesterday == day)
    {
        tr!("Yesterday, {time}", time = clock)
    } else if day.year() == now.date().year() {
        DATES.date_time(&time, false)
    } else {
        DATES.date_time(&time, true)
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
        .map(|timestamp| DATES.date_time(&timestamp.to_zoned(bande_datetime::local_zone()), true))
        .unwrap_or_default()
}

pub fn format_civil_date(time: jiff::civil::DateTime) -> String {
    time.to_zoned(bande_datetime::local_zone())
        .map(|time| DATES.date_time(&time, true))
        .unwrap_or_default()
}
