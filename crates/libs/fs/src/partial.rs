use std::fs::{self, File};
use std::io;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime};

use rustix::fs::{FlockOperation, fdatasync, flock, fsync, syncfs};

use crate::trash::remove_all;

const PREFIX: &str = ".bande-partial-";
const STALE_AFTER: Duration = Duration::from_secs(60);

static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn is_partial(name: &str) -> bool {
    name.starts_with(PREFIX)
}

pub(crate) fn partial_path(folder: &Path) -> PathBuf {
    let process = std::process::id();
    loop {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = folder.join(format!("{PREFIX}{process}-{number}"));
        if path.symlink_metadata().is_err() {
            return path;
        }
    }
}

#[derive(Debug)]
pub(crate) enum Anchor {
    File(OwnedFd),
    Folder(OwnedFd),
}

fn hold(fd: &OwnedFd) {
    let _ = flock(fd, FlockOperation::NonBlockingLockExclusive);
}

impl Anchor {
    pub(crate) fn file(file: &File) -> io::Result<Self> {
        let fd = OwnedFd::from(file.try_clone()?);
        hold(&fd);
        Ok(Self::File(fd))
    }

    pub(crate) fn folder(path: &Path) -> io::Result<Self> {
        let fd = OwnedFd::from(File::open(path)?);
        hold(&fd);
        Ok(Self::Folder(fd))
    }

    pub(crate) fn flush(&self) -> io::Result<()> {
        match self {
            Self::File(fd) => fdatasync(fd)?,
            Self::Folder(fd) => syncfs(fd)?,
        }
        Ok(())
    }

    pub(crate) fn settle(&self) -> io::Result<()> {
        match self {
            Self::File(fd) => fsync(fd)?,
            Self::Folder(fd) => syncfs(fd)?,
        }
        Ok(())
    }
}

fn remove_if_stale(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    let age = SystemTime::now()
        .duration_since(metadata.modified()?)
        .unwrap_or_default();
    if age < STALE_AFTER {
        return Ok(());
    }
    if metadata.is_symlink() {
        return remove_all(path);
    }
    let held = File::open(path)?;
    flock(&held, FlockOperation::NonBlockingLockExclusive)?;
    remove_all(path)
}

pub(crate) fn sweep(path: PathBuf) {
    let _ = thread::Builder::new()
        .name("stale partial".to_owned())
        .spawn(move || {
            let _ = remove_if_stale(&path);
        });
}
