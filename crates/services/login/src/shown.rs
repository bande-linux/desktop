use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;
use std::time::{Duration, Instant};

use rustix::fs::inotify::{self, CreateFlags, WatchFlags};
use rustix::fs::{AtFlags, Mode, OFlags};

/// Waits for a compositor to mark its first frame as shown.
pub struct ShownWatch {
    folder: OwnedFd,
    mark: &'static str,
    events: OwnedFd,
}

impl ShownWatch {
    /// Watches `runtime_dir` for `mark`, after removing one an earlier compositor left there.
    pub fn new(runtime_dir: &Path, mark: &'static str) -> io::Result<Self> {
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
        match rustix::fs::unlinkat(&folder, mark, AtFlags::empty()) {
            Ok(()) | Err(rustix::io::Errno::NOENT) => {}
            Err(error) => return Err(error.into()),
        }
        Ok(Self {
            folder,
            mark,
            events,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.events.as_fd()
    }

    /// What the runtime directory shows since the last look, consuming the events that woke the
    /// caller.
    pub fn progress(&self) -> io::Result<Progress> {
        let mut buffer = [MaybeUninit::uninit(); 4096];
        let mut reader = inotify::Reader::new(&self.events, &mut buffer);
        let mut progress = Progress::default();
        loop {
            match reader.next() {
                Ok(event) => {
                    progress.changed = true;
                    progress.compositor_started |= event
                        .file_name()
                        .and_then(|name| name.to_str().ok())
                        .is_some_and(is_wayland_socket);
                }
                Err(rustix::io::Errno::AGAIN) => break,
                Err(error) => return Err(error.into()),
            }
        }
        progress.shown =
            match rustix::fs::statat(&self.folder, self.mark, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(_) => true,
                Err(rustix::io::Errno::NOENT) => false,
                Err(error) => return Err(error.into()),
            };
        Ok(progress)
    }
}

fn is_wayland_socket(name: &str) -> bool {
    name.strip_prefix("wayland-")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit()))
}

/// What a session's runtime directory showed while its desktop was awaited.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// The compositor has shown its first frame.
    pub shown: bool,
    /// The compositor has opened its Wayland socket.
    pub compositor_started: bool,
    /// Anything appeared there.
    pub changed: bool,
}

/// How long a session may take to show its desktop. Until its compositor has started (the
/// profile and `bande-session` may be slow on a first login or a network home) only the
/// ceiling applies; after that the session is given up once nothing has appeared in its runtime
/// directory for a while, which is a compositor that has hung.
#[derive(Debug, Clone, Copy)]
pub struct FirstFrame {
    ceiling: Instant,
    quiet_since: Option<Instant>,
}

impl FirstFrame {
    /// The longest a session may take to show its desktop, however busy it looks.
    pub const CEILING: Duration = Duration::from_secs(300);
    /// How long a started compositor may go with nothing new in the runtime directory.
    pub const QUIET: Duration = Duration::from_secs(30);

    pub fn new(spawned: Instant) -> Self {
        Self {
            ceiling: spawned + Self::CEILING,
            quiet_since: None,
        }
    }

    pub fn saw(&mut self, progress: Progress, now: Instant) {
        if progress.compositor_started || (progress.changed && self.quiet_since.is_some()) {
            self.quiet_since = Some(now);
        }
    }

    pub fn deadline(&self) -> Instant {
        match self.quiet_since {
            Some(since) => (since + Self::QUIET).min(self.ceiling),
            None => self.ceiling,
        }
    }
}
