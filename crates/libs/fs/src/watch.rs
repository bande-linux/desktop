use std::collections::{BTreeSet, HashMap};
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use rustix::io::Errno;

const SETTLE: Duration = Duration::from_millis(80);
const BUFFER: usize = 16 * 1024;

#[derive(Debug)]
pub struct FolderWatch {
    fd: OwnedFd,
    watches: HashMap<i32, PathBuf>,
    changed: BTreeSet<PathBuf>,
    due: Option<Instant>,
}

impl FolderWatch {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            fd: inotify::init(CreateFlags::NONBLOCK | CreateFlags::CLOEXEC)?,
            watches: HashMap::new(),
            changed: BTreeSet::new(),
            due: None,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    pub fn watch(&mut self, folders: &[PathBuf]) {
        let stale = self
            .watches
            .iter()
            .filter(|(_, folder)| !folders.contains(folder))
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
            if self.watches.values().any(|known| known == folder) {
                continue;
            }
            if let Ok(wd) = inotify::add_watch(&self.fd, folder.as_path(), flags) {
                self.watches.insert(wd, folder.clone());
            }
        }
    }

    pub fn read(&mut self) {
        let mut buffer = [MaybeUninit::<u8>::uninit(); BUFFER];
        let mut reader = inotify::Reader::new(&self.fd, &mut buffer);
        let mut changed = Vec::new();
        let mut forgotten = Vec::new();
        let mut overflowed = false;
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
                    changed.push(event.wd());
                }
                Err(Errno::INTR) => {}
                Err(_) => break,
            }
        }
        if overflowed {
            self.changed.extend(self.watches.values().cloned());
        }
        self.changed.extend(
            changed
                .iter()
                .filter_map(|wd| self.watches.get(wd))
                .cloned(),
        );
        for wd in forgotten {
            self.watches.remove(&wd);
        }
        if !self.changed.is_empty() {
            self.due.get_or_insert_with(|| Instant::now() + SETTLE);
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.due
    }

    pub fn take_changed(&mut self, now: Instant) -> Vec<PathBuf> {
        if self.due.is_none_or(|due| due > now) {
            return Vec::new();
        }
        self.due = None;
        std::mem::take(&mut self.changed).into_iter().collect()
    }
}
