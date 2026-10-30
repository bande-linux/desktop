use std::path::{Path, PathBuf};

use unbound_xdg::{UserDir, path_from_file_uri, user_dir};

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

pub fn rows(home: &Path, user_dirs: Option<&str>, bookmarks: Option<&str>) -> Vec<Row> {
    let mut favourites = vec![Place {
        name: folder_name(home),
        location: Location::Folder(home.to_path_buf()),
        icon: "user-home",
        kind: PlaceKind::Standard,
    }];
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
    let mut rows = vec![Row::Heading(L10N.tr("sidebar-favourites"))];
    rows.extend(favourites.into_iter().map(Row::Place));
    rows.push(Row::Heading(L10N.tr("sidebar-locations")));
    rows.push(Row::Place(Place {
        name: L10N.tr("trash"),
        location: Location::Trash,
        icon: "user-trash",
        kind: PlaceKind::Standard,
    }));
    rows
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
