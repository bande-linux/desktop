use std::path::{Path, PathBuf};

use bande_xdg::{UserDir, file_uri, path_from_file_uri, user_dir};

use crate::L10N;
use crate::browser::Location;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceKind {
    Standard,
    Bookmark,
    Volume,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    pub location: Location,
    pub icon: &'static str,
    pub kind: PlaceKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Heading(String),
    Place(Place),
}

const USER_DIRS: [(UserDir, &str); 6] = [
    (UserDir::Desktop, "user-desktop"),
    (UserDir::Documents, "folder-documents"),
    (UserDir::Downloads, "folder-download"),
    (UserDir::Music, "folder-music"),
    (UserDir::Pictures, "folder-pictures"),
    (UserDir::Videos, "folder-videos"),
];

fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn bookmark(line: &str) -> Option<Place> {
    let (uri, label) = match line.split_once(' ') {
        Some((uri, label)) => (uri, Some(label.trim())),
        None => (line.trim(), None),
    };
    let path = path_from_file_uri(uri)?;
    let name = label
        .filter(|label| !label.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| folder_name(&path));
    Some(Place {
        name,
        location: Location::Folder(path),
        icon: "folder",
        kind: PlaceKind::Bookmark,
    })
}

const RECENTS: &str = "recents";
const TRASH: &str = "trash";
const VOLUMES: &str = "volumes";
const HIDDEN: char = '-';

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookmarkEdit {
    Add(Vec<PathBuf>),
    Remove(PathBuf),
    Move {
        folder: PathBuf,
        before: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarEdit {
    pub layout: Option<String>,
    pub bookmarks: Option<BookmarkEdit>,
}

fn recents() -> Place {
    Place {
        name: L10N.tr("recents"),
        location: Location::Recents,
        icon: "document-open-recent",
        kind: PlaceKind::Standard,
    }
}

fn trash() -> Place {
    Place {
        name: L10N.tr("trash"),
        location: Location::Trash,
        icon: "user-trash",
        kind: PlaceKind::Standard,
    }
}

pub fn standard_places(home: &Path, user_dirs: Option<&str>) -> Vec<Place> {
    let mut places = defaults(home, user_dirs, None);
    places.push(trash());
    places
}

pub fn volumes_hidden(layout: Option<&str>) -> bool {
    layout
        .into_iter()
        .flat_map(str::lines)
        .any(|line| line.strip_prefix(HIDDEN).map(str::trim) == Some(VOLUMES))
}

pub fn with_place_shown(layout: &str, place: Option<&Location>, shown: bool) -> String {
    let Some(token) = place.map_or_else(|| Some(VOLUMES.to_owned()), token) else {
        return layout.to_owned();
    };
    let hidden = format!("{HIDDEN}{token}");
    let mut lines = layout
        .lines()
        .filter(|line| line.trim() != hidden)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if !shown {
        lines.push(hidden);
    }
    lines.into_iter().map(|line| format!("{line}\n")).collect()
}

fn defaults(home: &Path, user_dirs: Option<&str>, bookmarks: Option<&str>) -> Vec<Place> {
    let mut favourites = vec![
        recents(),
        Place {
            name: folder_name(home),
            location: Location::Folder(home.to_path_buf()),
            icon: "user-home",
            kind: PlaceKind::Standard,
        },
    ];
    for (dir, icon) in USER_DIRS {
        let path = user_dir(home, user_dirs, dir);
        if path != home && path.is_dir() {
            favourites.push(Place {
                name: folder_name(&path),
                location: Location::Folder(path),
                icon,
                kind: PlaceKind::Standard,
            });
        }
    }
    for place in bookmarks
        .into_iter()
        .flat_map(str::lines)
        .filter_map(bookmark)
    {
        if !favourites
            .iter()
            .any(|known| known.location == place.location)
        {
            favourites.push(place);
        }
    }
    favourites
}

fn token(location: &Location) -> Option<String> {
    match location {
        Location::Recents => Some(RECENTS.to_owned()),
        Location::Trash => Some(TRASH.to_owned()),
        Location::Folder(path) => Some(file_uri(path)),
        Location::Search(_) => None,
    }
}

fn location(token: &str) -> Option<Location> {
    match token.trim() {
        RECENTS => Some(Location::Recents),
        TRASH => Some(Location::Trash),
        uri => bookmark(uri).map(|place| place.location),
    }
}

pub fn hidden_places(layout: Option<&str>) -> Vec<Location> {
    layout
        .into_iter()
        .flat_map(str::lines)
        .filter_map(|line| line.strip_prefix(HIDDEN))
        .filter_map(location)
        .collect()
}

pub fn rows(
    home: &Path,
    user_dirs: Option<&str>,
    bookmarks: Option<&str>,
    layout: Option<&str>,
) -> Vec<Row> {
    let mut known = defaults(home, user_dirs, bookmarks);
    let hidden = hidden_places(layout);
    let mut favourites = Vec::new();
    for line in layout.into_iter().flat_map(str::lines) {
        if line.starts_with(HIDDEN) {
            continue;
        }
        let Some(location) = location(line) else {
            continue;
        };
        if let Some(at) = known.iter().position(|place| place.location == location) {
            favourites.push(known.remove(at));
        }
    }
    favourites.extend(
        known
            .into_iter()
            .filter(|place| !hidden.contains(&place.location)),
    );
    let mut rows = vec![Row::Heading(L10N.tr("sidebar-favourites"))];
    rows.extend(favourites.into_iter().map(Row::Place));
    rows.push(Row::Heading(L10N.tr("sidebar-locations")));
    if !hidden.contains(&Location::Trash) {
        rows.push(Row::Place(trash()));
    }
    rows
}

pub fn favourites(rows: &[Row]) -> Vec<&Place> {
    rows.iter()
        .skip(1)
        .map_while(|row| match row {
            Row::Place(place) => Some(place),
            Row::Heading(_) => None,
        })
        .collect()
}

pub fn layout_text(favourites: &[&Place], hidden: &[Location]) -> String {
    let shown = favourites.iter().filter_map(|place| token(&place.location));
    let hidden = hidden
        .iter()
        .filter_map(token)
        .map(|token| format!("{HIDDEN}{token}"));
    shown
        .chain(hidden)
        .map(|line| format!("{line}\n"))
        .collect()
}

pub fn without_bookmark(bookmarks: &str, folder: &Path) -> String {
    bookmarks
        .lines()
        .filter(|line| {
            bookmark(line)
                .is_none_or(|place| place.location != Location::Folder(folder.to_path_buf()))
        })
        .map(|line| format!("{line}\n"))
        .collect()
}

pub fn with_bookmarks(bookmarks: &str, folders: &[PathBuf], before: Option<&Path>) -> String {
    let location = |line: &str| bookmark(line).map(|place| place.location);
    let is =
        |line: &str, folder: &Path| location(line) == Some(Location::Folder(folder.to_path_buf()));
    let lines = bookmarks
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    let added = folders
        .iter()
        .map(|folder| {
            lines
                .iter()
                .find(|line| is(line, folder))
                .map_or_else(|| file_uri(folder), |line| (*line).to_owned())
        })
        .collect::<Vec<_>>();
    let mut kept = lines
        .into_iter()
        .filter(|line| !folders.iter().any(|folder| is(line, folder)))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let at = before
        .and_then(|before| kept.iter().position(|line| is(line, before)))
        .unwrap_or(kept.len());
    kept.splice(at..at, added);
    kept.into_iter().map(|line| format!("{line}\n")).collect()
}

pub fn with_volumes(rows: &[Row], volumes: &[PathBuf]) -> Vec<Row> {
    let mut rows = rows
        .iter()
        .filter(|row| !matches!(row, Row::Place(place) if place.kind == PlaceKind::Volume))
        .cloned()
        .collect::<Vec<_>>();
    let at = rows
        .iter()
        .position(|row| matches!(row, Row::Place(place) if place.location == Location::Trash))
        .unwrap_or(rows.len());
    rows.splice(
        at..at,
        volumes.iter().map(|volume| {
            Row::Place(Place {
                name: folder_name(volume),
                location: Location::Folder(volume.clone()),
                icon: "drive-removable-media",
                kind: PlaceKind::Volume,
            })
        }),
    );
    rows
}
