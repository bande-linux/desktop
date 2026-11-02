use std::collections::HashMap;
use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::thread;

use bande_inbox::{Inbox, Outbox};
use bande_watch::{Changes, FolderWatch};
use bande_xdg::path_from_file_uri;

use crate::usage::Usage;

pub const FILE_KEY: &str = "file:";

pub fn recently_used_path(data_home: Option<&Path>, home: &Path) -> PathBuf {
    data_home
        .filter(|path| path.is_absolute())
        .map_or_else(|| home.join(".local/share"), Path::to_path_buf)
        .join("recently-used.xbel")
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let length = tag[start..].find('"')?;
    Some(&tag[start..start + length])
}

fn unescape(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn seconds(stamp: &str) -> Option<u64> {
    let stamp = stamp.parse::<jiff::Timestamp>().ok()?;
    u64::try_from(stamp.as_second()).ok()
}

pub fn recently_used(xbel: &str) -> Vec<(PathBuf, u64)> {
    xbel.split("<bookmark ")
        .skip(1)
        .filter_map(|rest| {
            let tag = format!(" {}", &rest[..rest.find('>')?]);
            let tag = tag.as_str();
            let path = path_from_file_uri(&unescape(attribute(tag, "href")?))?;
            let used = ["visited", "modified", "added"]
                .iter()
                .filter_map(|name| attribute(tag, name).and_then(seconds))
                .max()?;
            Some((path, used))
        })
        .collect()
}

pub type Recents = Vec<(PathBuf, u64)>;

pub struct RecentlyUsed {
    _watch: FolderWatch,
    read: Inbox<Recents>,
    recents: Recents,
}

fn read_recents(path: &Path, outbox: &Outbox<Recents>) {
    let xbel = std::fs::read_to_string(path).unwrap_or_default();
    outbox.send(recently_used(&xbel));
}

impl RecentlyUsed {
    pub fn follow(path: PathBuf) -> io::Result<Self> {
        let read = Inbox::new()?;
        let (outbox, first) = (read.outbox(), read.outbox());
        let watched = path.clone();
        let watch = FolderWatch::spawn(Changes::Saved, move |changed| {
            if changed.contains(&watched) {
                read_recents(&watched, &outbox);
            }
        })?;
        if let Some(folder) = path.parent() {
            watch.watch(folder)?;
        }
        thread::Builder::new()
            .name("recently-used".to_owned())
            .spawn(move || read_recents(&path, &first))?;
        Ok(Self {
            _watch: watch,
            read,
            recents: Vec::new(),
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.read.fd()
    }

    pub fn take(&mut self) -> bool {
        match self.read.drain().pop() {
            Some(recents) => {
                self.recents = recents;
                true
            }
            None => false,
        }
    }

    pub fn recents(&self) -> &[(PathBuf, u64)] {
        &self.recents
    }
}

pub fn is_excluded(path: &Path, excluded: &[PathBuf]) -> bool {
    excluded.iter().any(|folder| path.starts_with(folder))
}

pub fn recent_files(
    recents: &[(PathBuf, u64)],
    usage: &Usage,
    excluded: &[PathBuf],
    limit: usize,
) -> Vec<PathBuf> {
    let mut latest: HashMap<PathBuf, u64> = HashMap::new();
    let used = usage.last_used(FILE_KEY).chain(recents.iter().cloned());
    for (path, at) in used {
        let known = latest.entry(path).or_insert(at);
        *known = (*known).max(at);
    }
    let mut ordered = latest.into_iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ordered
        .into_iter()
        .map(|(path, _)| path)
        .filter(|path| !is_excluded(path, excluded) && path.is_file())
        .take(limit)
        .collect()
}
