use std::collections::{BTreeSet, HashMap};
use std::ffi::OsStr;
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use rustix::io::Errno;

const SETTLE: Duration = Duration::from_millis(80);
const BUFFER: usize = 16 * 1024;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct FolderChanges {
    pub folders: Vec<PathBuf>,
    pub entries: Vec<PathBuf>,
}

impl FolderChanges {
    pub fn is_empty(&self) -> bool {
        self.folders.is_empty() && self.entries.is_empty()
    }
}

#[derive(Debug)]
pub struct FolderWatch {
    fd: OwnedFd,
    watches: HashMap<i32, Vec<PathBuf>>,
    changed: BTreeSet<PathBuf>,
    touched: BTreeSet<PathBuf>,
    due: Option<Instant>,
}

impl FolderWatch {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            fd: inotify::init(CreateFlags::NONBLOCK | CreateFlags::CLOEXEC)?,
            watches: HashMap::new(),
            changed: BTreeSet::new(),
            touched: BTreeSet::new(),
            due: None,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    pub fn watch(&mut self, folders: &[PathBuf]) {
        for paths in self.watches.values_mut() {
            paths.retain(|path| folders.contains(path));
        }
        let stale = self
            .watches
            .iter()
            .filter(|(_, paths)| paths.is_empty())
            .map(|(wd, _)| *wd)
            .collect::<Vec<_>>();
        for wd in stale {
            let _ = inotify::remove_watch(&self.fd, wd);
            self.watches.remove(&wd);
        }
        let flags = WatchFlags::CREATE
            | WatchFlags::DELETE
            | WatchFlags::MOVED_FROM
            | WatchFlags::MOVED_TO
            | WatchFlags::CLOSE_WRITE
            | WatchFlags::ATTRIB
            | WatchFlags::DELETE_SELF
            | WatchFlags::MOVE_SELF
            | WatchFlags::ONLYDIR;
        for folder in folders {
            if self.watches.values().flatten().any(|known| known == folder) {
                continue;
            }
            if let Ok(wd) = inotify::add_watch(&self.fd, folder.as_path(), flags) {
                self.watches.entry(wd).or_default().push(folder.clone());
            }
        }
    }

    pub fn read(&mut self) {
        let mut buffer = [MaybeUninit::<u8>::uninit(); BUFFER];
        let mut reader = inotify::Reader::new(&self.fd, &mut buffer);
        let mut changed = Vec::new();
        let mut touched = Vec::new();
        let mut forgotten = Vec::new();
        let mut overflowed = false;
        let whole = ReadFlags::QUEUE_OVERFLOW
            | ReadFlags::IGNORED
            | ReadFlags::DELETE_SELF
            | ReadFlags::MOVE_SELF;
        loop {
            match reader.next() {
                Ok(event) => {
                    let flags = event.events();
                    if flags.contains(ReadFlags::QUEUE_OVERFLOW) {
                        overflowed = true;
                    }
                    if flags.contains(ReadFlags::IGNORED) {
                        forgotten.push(event.wd());
                    }
                    let name = event
                        .file_name()
                        .map(|name| OsStr::from_bytes(name.to_bytes()).to_os_string())
                        .filter(|name| !name.is_empty());
                    match name {
                        Some(name) if !flags.intersects(whole) => touched.push((event.wd(), name)),
                        _ => changed.push(event.wd()),
                    }
                }
                Err(Errno::INTR) => {}
                Err(_) => break,
            }
        }
        if overflowed {
            self.changed
                .extend(self.watches.values().flatten().cloned());
        }
        self.changed.extend(
            changed
                .iter()
                .filter_map(|wd| self.watches.get(wd))
                .flatten()
                .cloned(),
        );
        for (wd, name) in touched {
            for folder in self.watches.get(&wd).into_iter().flatten() {
                self.touched.insert(folder.join(&name));
            }
        }
        for wd in forgotten {
            self.watches.remove(&wd);
        }
        if !self.changed.is_empty() || !self.touched.is_empty() {
            self.due.get_or_insert_with(|| Instant::now() + SETTLE);
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.due
    }

    pub fn take_changed(&mut self, now: Instant) -> FolderChanges {
        if self.due.is_none_or(|due| due > now) {
            return FolderChanges::default();
        }
        self.due = None;
        let folders = std::mem::take(&mut self.changed);
        let entries = std::mem::take(&mut self.touched)
            .into_iter()
            .filter(|path| path.parent().is_none_or(|folder| !folders.contains(folder)))
            .collect();
        FolderChanges {
            folders: folders.into_iter().collect(),
            entries,
        }
    }
}
