use std::collections::HashMap;
use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::thread;

use bande_inbox::{Inbox, Outbox};
use bande_watch::{Changes, FolderWatch};
use bande_xdg::{path_from_file_uri, unescape_xml};

use crate::usage::Usage;

pub const FILE_KEY: &str = "file:";

pub fn recently_used_path() -> Option<PathBuf> {
    bande_xdg::data_home().map(|data| data.join("recently-used.xbel"))
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let length = tag[start..].find('"')?;
    Some(&tag[start..start + length])
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
            let path = path_from_file_uri(&unescape_xml(attribute(tag, "href")?))?;
            let used = ["visited", "modified", "added"]
                .iter()
                .filter_map(|name| attribute(tag, name).and_then(seconds))
                .max()?;
            Some((path, used))
        })
        .collect()
}

const XBEL_START: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xbel version=\"1.0\"\n      \
     xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\"\n      \
     xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\"\n>\n";
const XBEL_END: &str = "</xbel>";
const APPLICATIONS_END: &str = "</bookmark:applications>";

fn with_attribute(tag: &str, name: &str, value: &str) -> String {
    let key = format!(" {name}=\"");
    match tag.find(&key) {
        Some(start) => {
            let from = start + key.len();
            let end = tag[from..]
                .find('"')
                .map_or(tag.len(), |length| from + length);
            format!("{}{value}{}", &tag[..from], &tag[end..])
        }
        None => {
            let at = tag
                .rfind("/>")
                .or_else(|| tag.rfind('>'))
                .unwrap_or(tag.len());
            format!("{} {name}=\"{value}\"{}", tag[..at].trim_end(), &tag[at..])
        }
    }
}

fn application_tag(name: &str, exec: &str, now: &str, count: u64) -> String {
    format!(
        "<bookmark:application name=\"{}\" exec=\"{}\" modified=\"{now}\" count=\"{count}\"/>",
        bande_xdg::escape_xml(name),
        bande_xdg::escape_xml(&format!("'{exec}'")),
    )
}

fn used_again(bookmark: &str, name: &str, exec: &str, now: &str) -> String {
    let open_end = bookmark.find('>').map_or(bookmark.len(), |end| end + 1);
    let (open, mut rest) = (
        bookmark[..open_end].to_owned(),
        bookmark[open_end..].to_owned(),
    );
    let open = with_attribute(&with_attribute(&open, "modified", now), "visited", now);
    let named = format!(
        "<bookmark:application name=\"{}\"",
        bande_xdg::escape_xml(name)
    );
    match rest.find(&named) {
        Some(start) => {
            let end = rest[start..]
                .find("/>")
                .map_or(rest.len(), |length| start + length + 2);
            let count = attribute(&rest[start..end], "count")
                .and_then(|count| count.parse::<u64>().ok())
                .unwrap_or(0);
            rest.replace_range(start..end, &application_tag(name, exec, now, count + 1));
        }
        None => match rest.find(APPLICATIONS_END) {
            Some(at) => rest.insert_str(at, &application_tag(name, exec, now, 1)),
            None => tracing::debug!("a recently used file lists no applications"),
        },
    }
    open + &rest
}

fn new_bookmark(href: &str, mime: &str, name: &str, exec: &str, now: &str) -> String {
    format!(
        "  <bookmark href=\"{href}\" added=\"{now}\" modified=\"{now}\" visited=\"{now}\">\n    \
         <info>\n      <metadata owner=\"http://freedesktop.org\">\n        \
         <mime:mime-type type=\"{}\"/>\n        <bookmark:applications>\n          {}\n        \
         </bookmark:applications>\n      </metadata>\n    </info>\n  </bookmark>\n",
        bande_xdg::escape_xml(mime),
        application_tag(name, exec, now, 1),
    )
}

fn with_recently_used(
    xbel: &str,
    files: &[(PathBuf, String)],
    name: &str,
    exec: &str,
) -> Option<String> {
    let now = jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%S%.6fZ")
        .to_string();
    let mut xbel = match (xbel.contains(XBEL_END), xbel.trim().is_empty()) {
        (true, _) => xbel.to_owned(),
        (false, true) => format!("{XBEL_START}{XBEL_END}\n"),
        (false, false) => return None,
    };
    for (path, mime) in files {
        let href = bande_xdg::escape_xml(&bande_xdg::file_uri(path));
        let opening = format!("<bookmark href=\"{href}\"");
        match xbel.find(&opening) {
            Some(start) => {
                let end = xbel[start..]
                    .find("</bookmark>")
                    .map_or(xbel.len(), |length| start + length);
                let updated = used_again(&xbel[start..end], name, exec, &now);
                xbel.replace_range(start..end, &updated);
            }
            None => {
                let at = xbel.rfind(XBEL_END).unwrap_or(xbel.len());
                xbel.insert_str(at, &new_bookmark(&href, mime, name, exec, &now));
            }
        }
    }
    Some(xbel)
}

/// Whether the user lets the desktop remember recently used files.
pub fn remembers_recent_files() -> bool {
    bande_config::config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|source| bande_config::Config::parse(&source).ok())
        .is_none_or(|config| config.remember_recent_files)
}

/// Adds `files` to `recently-used.xbel` as opened with the app `name` that `exec` runs, unless
/// remembering recent files is off. Works on the thread it returns.
pub fn add_recently_used(
    files: Vec<(PathBuf, String)>,
    name: String,
    exec: String,
) -> Option<thread::JoinHandle<()>> {
    let work = move || {
        if !remembers_recent_files() {
            return;
        }
        let Some(path) = recently_used_path() else {
            return;
        };
        let xbel = match std::fs::read_to_string(&path) {
            Ok(xbel) => xbel,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                tracing::warn!("cannot read {}: {error}", path.display());
                return;
            }
        };
        let Some(updated) = with_recently_used(&xbel, &files, &name, &exec) else {
            tracing::warn!(
                "{} ends early, so it is left as it is and these files are not added",
                path.display()
            );
            return;
        };
        if let Err(error) = bande_xdg::replace_file(&path, updated) {
            tracing::warn!("cannot remember the recently opened files: {error}");
        }
    };
    thread::Builder::new()
        .name("recently-used".to_owned())
        .spawn(work)
        .inspect_err(|error| tracing::warn!("cannot remember the recently opened files: {error}"))
        .ok()
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
        .filter(|path| {
            !is_excluded(path, excluded)
                && (bande_volumes::on_connected_volume(path) || path.is_file())
        })
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{recently_used, with_recently_used};

    #[test]
    fn opening_files_again_keeps_one_bookmark_each_and_counts_the_app() {
        let report = (
            PathBuf::from("/home/a/R&D <draft>.odt"),
            "application/x-test".to_owned(),
        );
        let notes = (PathBuf::from("/home/a/notes.txt"), "text/plain".to_owned());
        let once = with_recently_used(
            "",
            std::slice::from_ref(&report),
            "Text Editor",
            "bande-text-editor %U",
        )
        .expect("an empty file starts a new list");
        let twice = with_recently_used(
            &once,
            &[report.clone(), notes.clone()],
            "Text Editor",
            "bande-text-editor %U",
        )
        .expect("a whole file takes more files");
        let paths: Vec<PathBuf> = recently_used(&twice)
            .into_iter()
            .map(|(path, _)| path)
            .collect();
        assert_eq!(paths, vec![report.0, notes.0]);
        assert_eq!(twice.matches("count=\"2\"").count(), 1);
        assert_eq!(twice.matches("<bookmark:application ").count(), 2);
        assert!(twice.ends_with("</xbel>\n"));
    }
}
