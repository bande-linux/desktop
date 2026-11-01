use std::collections::HashMap;
use std::ffi::OsStr;
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use rustix::event::{PollFd, PollFlags, poll};
use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use rustix::io::Errno;
use rustix::pipe::{PipeFlags, pipe_with};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Changes {
    Saved,
    Any,
}

impl Changes {
    fn flags(self) -> WatchFlags {
        match self {
            Self::Saved => WatchFlags::CLOSE_WRITE | WatchFlags::MOVED_TO,
            Self::Any => {
                WatchFlags::CREATE
                    | WatchFlags::DELETE
                    | WatchFlags::CLOSE_WRITE
                    | WatchFlags::MOVED_FROM
                    | WatchFlags::MOVED_TO
                    | WatchFlags::DELETE_SELF
                    | WatchFlags::MOVE_SELF
            }
        }
    }
}

type Folders = Arc<Mutex<HashMap<i32, Vec<PathBuf>>>>;

pub struct FolderWatch {
    inotify: Arc<OwnedFd>,
    folders: Folders,
    changes: Changes,
    _stop: OwnedFd,
}

fn lock(folders: &Folders) -> MutexGuard<'_, HashMap<i32, Vec<PathBuf>>> {
    folders.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FolderWatch {
    pub fn spawn(
        changes: Changes,
        mut on_change: impl FnMut(Vec<PathBuf>) + Send + 'static,
    ) -> io::Result<Self> {
        let inotify = Arc::new(inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)?);
        let (stopped, stop) = pipe_with(PipeFlags::CLOEXEC)?;
        let folders = Folders::default();
        let reader = (Arc::clone(&inotify), Arc::clone(&folders));
        thread::Builder::new()
            .name("folder-watch".to_owned())
            .spawn(move || {
                let (inotify, folders) = reader;
                while let Some(paths) = next_changes(&inotify, &stopped, &folders) {
                    if !paths.is_empty() {
                        on_change(paths);
                    }
                }
            })?;
        Ok(Self {
            inotify,
            folders,
            changes,
            _stop: stop,
        })
    }

    pub fn watch(&self, folder: &Path) -> io::Result<()> {
        let mut folders = lock(&self.folders);
        let descriptor = inotify::add_watch(
            self.inotify.as_ref(),
            folder,
            self.changes.flags() | WatchFlags::ONLYDIR,
        )?;
        let paths = folders.entry(descriptor).or_default();
        if !paths.iter().any(|path| path == folder) {
            paths.push(folder.to_path_buf());
        }
        Ok(())
    }
}

fn next_changes(inotify: &OwnedFd, stopped: &OwnedFd, folders: &Folders) -> Option<Vec<PathBuf>> {
    let mut fds = [
        PollFd::new(inotify, PollFlags::IN),
        PollFd::new(stopped, PollFlags::IN),
    ];
    match poll(&mut fds, None) {
        Ok(_) | Err(Errno::INTR) => {}
        Err(_) => return None,
    }
    if !fds[1].revents().is_empty() {
        return None;
    }
    let mut buffer = [MaybeUninit::uninit(); 8192];
    let mut reader = inotify::Reader::new(inotify, &mut buffer);
    let mut paths = Vec::new();
    loop {
        let event = match reader.next() {
            Ok(event) => event,
            Err(Errno::AGAIN | Errno::INTR) => return Some(paths),
            Err(_) => return None,
        };
        let mut folders = lock(folders);
        if event.events().contains(ReadFlags::IGNORED) {
            folders.remove(&event.wd());
            continue;
        }
        let Some(watched) = folders.get(&event.wd()) else {
            continue;
        };
        paths.extend(watched.iter().map(|folder| match event.file_name() {
            Some(name) => folder.join(OsStr::from_bytes(name.to_bytes())),
            None => folder.clone(),
        }));
    }
}
