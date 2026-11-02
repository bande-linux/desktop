use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::SystemTime;

use bande_xdg::{Apps, MimeDatabase, config_dirs, data_dirs};
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, Zoned};

use crate::L10N;
use crate::listing::Entry;

pub const ARCHIVES: &[&str] = &[
    "application/zip",
    "application/gzip",
    "application/x-tar",
    "application/x-7z-compressed",
    "application/x-rar",
    "application/vnd.rar",
    "application/x-xz",
    "application/x-bzip2",
    "application/zstd",
    "application/x-compressed-tar",
    "application/x-xz-compressed-tar",
    "application/x-bzip2-compressed-tar",
    "application/x-zstd-compressed-tar",
];
pub const DOCUMENTS: &[&str] = &[
    "application/pdf",
    "application/rtf",
    "application/msword",
    "application/epub+zip",
    "application/vnd.oasis.opendocument.",
    "application/vnd.openxmlformats-officedocument.",
    "application/vnd.ms-",
];
const PRESENTATIONS: &[&str] = &[
    "application/vnd.oasis.opendocument.presentation",
    "application/vnd.openxmlformats-officedocument.presentationml",
    "application/vnd.ms-powerpoint",
];
const SPREADSHEETS: &[&str] = &[
    "application/vnd.oasis.opendocument.spreadsheet",
    "application/vnd.openxmlformats-officedocument.spreadsheetml",
    "application/vnd.ms-excel",
    "text/csv",
];
const DISK_IMAGES: &[&str] = &[
    "application/x-cd-image",
    "application/x-raw-disk-image",
    "application/x-apple-diskimage",
    "application/vnd.efi.iso",
];
const APPLICATIONS: &[&str] = &[
    "application/x-desktop",
    "application/vnd.appimage",
    "application/x-executable",
];
const KILOBYTE: u64 = 1_000;
const MEGABYTE: u64 = 1_000_000;
const GIGABYTE: u64 = 1_000_000_000;
const YEARS_BACK: i16 = 60;

static MIME: LazyLock<MimeDatabase> = LazyLock::new(|| MimeDatabase::load(&data_dirs()));
static APPS: LazyLock<Apps> = LazyLock::new(|| Apps::load(data_dirs(), &config_dirs()));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GroupBy {
    Kind,
    Application,
    Modified,
    Created,
    Added,
    Size,
}

impl GroupBy {
    pub const ALL: [Self; 6] = [
        Self::Kind,
        Self::Application,
        Self::Modified,
        Self::Created,
        Self::Added,
        Self::Size,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Kind => "kind",
            Self::Application => "application",
            Self::Modified => "modified",
            Self::Created => "created",
            Self::Added => "added",
            Self::Size => "size",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|by| by.name() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KindGroup {
    Applications,
    Archives,
    DiskImages,
    Documents,
    Folders,
    Images,
    Movies,
    Music,
    PdfDocuments,
    Presentations,
    Spreadsheets,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DateGroup {
    Today,
    Yesterday,
    PreviousWeek,
    PreviousMonth,
    Month(Reverse<i8>),
    Year(Reverse<i16>),
    Undated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SizeGroup {
    OverGigabyte,
    HundredsOfMegabytes,
    Megabytes,
    HundredsOfKilobytes,
    UnderHundredKilobytes,
    Empty,
    Folders,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AppGroup {
    Named(&'static str),
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    Kind(KindGroup),
    Application(AppGroup),
    Date(DateGroup),
    Size(SizeGroup),
}

fn starts_with_any(mime: &str, known: &[&str]) -> bool {
    known.iter().any(|known| mime.starts_with(known))
}

thread_local! {
    static MIMES: RefCell<HashMap<String, Option<&'static str>>> = RefCell::new(HashMap::new());
    static OPENERS: RefCell<HashMap<&'static str, AppGroup>> = RefCell::new(HashMap::new());
}

fn mime_of(entry: &Entry) -> Option<&'static str> {
    let key = match entry.name.split_once('.') {
        Some((stem, suffix)) if !stem.is_empty() => suffix,
        _ => &entry.name,
    };
    MIMES.with_borrow_mut(|mimes| match mimes.get(key) {
        Some(mime) => *mime,
        None => {
            let mime = MIME.for_name(&entry.name);
            mimes.insert(key.to_owned(), mime);
            mime
        }
    })
}

fn kind_group(entry: &Entry) -> KindGroup {
    if entry.is_folder() {
        return KindGroup::Folders;
    }
    let Some(mime) = mime_of(entry) else {
        return KindGroup::Other;
    };
    match mime {
        "application/pdf" => KindGroup::PdfDocuments,
        _ if mime.starts_with("image/") => KindGroup::Images,
        _ if mime.starts_with("video/") => KindGroup::Movies,
        _ if mime.starts_with("audio/") => KindGroup::Music,
        _ if starts_with_any(mime, PRESENTATIONS) => KindGroup::Presentations,
        _ if starts_with_any(mime, SPREADSHEETS) => KindGroup::Spreadsheets,
        _ if ARCHIVES.contains(&mime) => KindGroup::Archives,
        _ if DISK_IMAGES.contains(&mime) => KindGroup::DiskImages,
        _ if APPLICATIONS.contains(&mime) => KindGroup::Applications,
        _ if mime.starts_with("text/") || starts_with_any(mime, DOCUMENTS) => KindGroup::Documents,
        _ => KindGroup::Other,
    }
}

fn app_group(entry: &Entry) -> AppGroup {
    if entry.is_folder() {
        return AppGroup::Other;
    }
    let Some(mime) = mime_of(entry) else {
        return AppGroup::Other;
    };
    OPENERS.with_borrow_mut(|openers| {
        *openers.entry(mime).or_insert_with(|| {
            APPS.default_for(mime)
                .map_or(AppGroup::Other, |app| AppGroup::Named(app.name.as_str()))
        })
    })
}

fn size_group(entry: &Entry) -> SizeGroup {
    match entry.size {
        _ if entry.is_folder() => SizeGroup::Folders,
        0 => SizeGroup::Empty,
        size if size < 100 * KILOBYTE => SizeGroup::UnderHundredKilobytes,
        size if size < MEGABYTE => SizeGroup::HundredsOfKilobytes,
        size if size < 100 * MEGABYTE => SizeGroup::Megabytes,
        size if size < GIGABYTE => SizeGroup::HundredsOfMegabytes,
        _ => SizeGroup::OverGigabyte,
    }
}

struct Calendar {
    tomorrow: SystemTime,
    days: [(SystemTime, DateGroup); 4],
    months: Vec<(SystemTime, DateGroup)>,
    years: Vec<(SystemTime, DateGroup)>,
}

fn start(zoned: Option<Zoned>) -> SystemTime {
    zoned.map_or(SystemTime::UNIX_EPOCH, |zoned| {
        SystemTime::from(zoned.timestamp())
    })
}

impl Calendar {
    fn new(now: SystemTime, zone: &TimeZone) -> Option<Self> {
        let today = Timestamp::try_from(now)
            .ok()?
            .to_zoned(zone.clone())
            .start_of_day()
            .ok()?;
        let back = |days: i64| today.checked_sub(Span::new().days(days)).ok();
        let first_of_year = today.first_of_year().ok()?;
        let months = (1..=today.month())
            .rev()
            .map(|month| {
                let first = first_of_year.with().month(month).build().ok();
                (start(first), DateGroup::Month(Reverse(month)))
            })
            .collect();
        let years = (1..=YEARS_BACK)
            .map(|back| {
                let year = today.year() - back;
                let first = first_of_year.with().year(year).build().ok();
                (start(first), DateGroup::Year(Reverse(year)))
            })
            .collect();
        Some(Self {
            tomorrow: start(today.checked_add(Span::new().days(1)).ok()),
            days: [
                (start(Some(today.clone())), DateGroup::Today),
                (start(back(1)), DateGroup::Yesterday),
                (start(back(7)), DateGroup::PreviousWeek),
                (start(back(30)), DateGroup::PreviousMonth),
            ],
            months,
            years,
        })
    }

    fn group(&self, time: SystemTime) -> DateGroup {
        self.days
            .iter()
            .chain(&self.months)
            .chain(&self.years)
            .find(|(start, _)| time >= *start)
            .map_or(DateGroup::Undated, |(_, group)| *group)
    }
}

thread_local! {
    static CALENDAR: RefCell<Option<Calendar>> = const { RefCell::new(None) };
}

fn date_group(time: Option<SystemTime>) -> DateGroup {
    let Some(time) = time else {
        return DateGroup::Undated;
    };
    CALENDAR.with_borrow_mut(|calendar| {
        let now = SystemTime::now();
        if calendar.as_ref().is_none_or(|known| now >= known.tomorrow) {
            *calendar = Calendar::new(now, &bande_datetime::local_zone());
        }
        calendar
            .as_ref()
            .map_or(DateGroup::Undated, |calendar| calendar.group(time))
    })
}

pub fn group_of(entry: &Entry, by: GroupBy) -> Group {
    match by {
        GroupBy::Kind => Group::Kind(kind_group(entry)),
        GroupBy::Application => Group::Application(app_group(entry)),
        GroupBy::Modified => Group::Date(date_group(entry.modified)),
        GroupBy::Created => Group::Date(date_group(entry.created.or(entry.modified))),
        GroupBy::Added => Group::Date(date_group(entry.added)),
        GroupBy::Size => Group::Size(size_group(entry)),
    }
}

impl Group {
    pub fn title(self) -> String {
        let message = match self {
            Self::Kind(kind) => match kind {
                KindGroup::Applications => "group-applications",
                KindGroup::Archives => "group-archives",
                KindGroup::DiskImages => "group-disk-images",
                KindGroup::Documents => "group-documents",
                KindGroup::Folders => "group-folders",
                KindGroup::Images => "group-images",
                KindGroup::Movies => "group-movies",
                KindGroup::Music => "group-music",
                KindGroup::PdfDocuments => "group-pdf-documents",
                KindGroup::Presentations => "group-presentations",
                KindGroup::Spreadsheets => "group-spreadsheets",
                KindGroup::Other => "group-other",
            },
            Self::Application(AppGroup::Named(name)) => return name.to_owned(),
            Self::Application(AppGroup::Other) => "group-other",
            Self::Date(date) => match date {
                DateGroup::Today => "group-today",
                DateGroup::Yesterday => "group-yesterday",
                DateGroup::PreviousWeek => "group-previous-week",
                DateGroup::PreviousMonth => "group-previous-month",
                DateGroup::Month(Reverse(month)) => {
                    return jiff::civil::date(2000, month, 1).strftime("%B").to_string();
                }
                DateGroup::Year(Reverse(year)) => return year.to_string(),
                DateGroup::Undated => "group-undated",
            },
            Self::Size(size) => match size {
                SizeGroup::OverGigabyte => "group-over-gigabyte",
                SizeGroup::HundredsOfMegabytes => "group-hundreds-of-megabytes",
                SizeGroup::Megabytes => "group-megabytes",
                SizeGroup::HundredsOfKilobytes => "group-hundreds-of-kilobytes",
                SizeGroup::UnderHundredKilobytes => "group-under-hundred-kilobytes",
                SizeGroup::Empty => "group-empty",
                SizeGroup::Folders => "group-folders",
            },
        };
        L10N.tr(message)
    }
}
