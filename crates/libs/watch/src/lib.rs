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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    use super::{Changes, FolderWatch};

    fn watching(
        changes: Changes,
        folder: &std::path::Path,
    ) -> (FolderWatch, Receiver<Vec<std::path::PathBuf>>) {
        let (sender, received) = channel();
        let watch = FolderWatch::spawn(changes, move |paths| {
            let _ = sender.send(paths);
        })
        .unwrap();
        watch.watch(folder).unwrap();
        (watch, received)
    }

    #[test]
    fn a_file_saved_into_a_watched_folder_is_reported_by_path() {
        let folder = tempfile::tempdir().unwrap();
        let (_watch, changes) = watching(Changes::Saved, folder.path());
        let staged = folder.path().join(".config.kdl.new");
        fs::write(&staged, "dock {}").unwrap();
        fs::rename(&staged, folder.path().join("config.kdl")).unwrap();
        let mut seen = Vec::new();
        while !seen.contains(&folder.path().join("config.kdl")) {
            seen.extend(changes.recv_timeout(Duration::from_secs(5)).unwrap());
        }
        assert!(seen.contains(&staged));
    }

    #[test]
    fn creating_and_removing_entries_is_reported_for_every_watched_folder() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("Documents");
        fs::create_dir(&nested).unwrap();
        let (watch, changes) = watching(Changes::Any, root.path());
        watch.watch(&nested).unwrap();
        fs::write(nested.join("notes.txt"), "").unwrap();
        fs::remove_dir_all(&nested).unwrap();
        let mut seen = Vec::new();
        while !seen.contains(&nested) {
            seen.extend(changes.recv_timeout(Duration::from_secs(5)).unwrap());
        }
        assert!(seen.contains(&nested.join("notes.txt")));
    }

    #[test]
    fn a_folder_watched_under_two_paths_reports_changes_under_both() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("Projects");
        let alias = root.path().join("Work");
        fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let (watch, changes) = watching(Changes::Saved, &real);
        watch.watch(&alias).unwrap();
        fs::write(real.join("plan.txt"), "draft").unwrap();
        let mut seen = Vec::new();
        while !(seen.contains(&real.join("plan.txt")) && seen.contains(&alias.join("plan.txt"))) {
            seen.extend(changes.recv_timeout(Duration::from_secs(5)).unwrap());
        }
    }

    #[test]
    fn a_saved_watch_ignores_new_empty_files() {
        let folder = tempfile::tempdir().unwrap();
        let (_watch, changes) = watching(Changes::Saved, folder.path());
        fs::create_dir(folder.path().join("cache")).unwrap();
        assert!(changes.recv_timeout(Duration::from_millis(200)).is_err());
    }
}
