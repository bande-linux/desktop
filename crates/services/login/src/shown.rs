use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;

use bande_xdg::DESKTOP_SHOWN;
use rustix::fs::inotify::{self, CreateFlags, WatchFlags};
use rustix::fs::{AtFlags, Mode, OFlags};

/// Waits for a session's compositor to show its first frame.
pub struct ShownWatch {
    folder: OwnedFd,
    events: OwnedFd,
}

impl ShownWatch {
    /// Watches `runtime_dir`, after removing a mark an earlier session left there.
    pub fn new(runtime_dir: &Path) -> io::Result<Self> {
        let folder = rustix::fs::open(
            runtime_dir,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let events = inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)?;
        inotify::add_watch(
            &events,
            runtime_dir,
            WatchFlags::CREATE
                | WatchFlags::MOVED_TO
                | WatchFlags::ONLYDIR
                | WatchFlags::DONT_FOLLOW,
        )?;
        match rustix::fs::unlinkat(&folder, DESKTOP_SHOWN, AtFlags::empty()) {
            Ok(()) | Err(rustix::io::Errno::NOENT) => {}
            Err(error) => return Err(error.into()),
        }
        Ok(Self { folder, events })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.events.as_fd()
    }

    /// Answers whether the mark is there, consuming the events that woke the caller.
    pub fn is_shown(&self) -> io::Result<bool> {
        let mut buffer = [0u8; 4096];
        loop {
            match rustix::io::read(&self.events, &mut buffer) {
                Ok(0) | Err(rustix::io::Errno::AGAIN) => break,
                Ok(_) => {}
                Err(error) => return Err(error.into()),
            }
        }
        match rustix::fs::statat(&self.folder, DESKTOP_SHOWN, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => Ok(true),
            Err(rustix::io::Errno::NOENT) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
}
