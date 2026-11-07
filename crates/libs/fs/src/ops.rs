use std::ffi::{OsStr, OsString};
use std::fs::{self, File, FileTimes, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, symlink};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use bande_xdg::sync_folder;
use rustix::fs::{XattrFlags, fgetxattr, flistxattr, fsetxattr, fstatfs};
use rustix::io::Errno;

mod archive;

pub use archive::{Contents, Format, Refusal, archive_contents, archive_path, can_extract};

use crate::L10N;
use crate::listing::same_volume;
use crate::naming::{alias_name, duplicate_name, keep_both_name, untitled_folder};
use crate::partial::{Anchor, partial_path};
use crate::trash::{Trash, TrashItem, erase, forget, remove_all, restore_to};

const CHUNK: u64 = 8 * 1024 * 1024;
const FLUSH_EVERY: u64 = 64 * 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);
const FAT_FILE_LIMIT: u64 = 0xFFFF_FFFF;
const FAT_MAGIC: i32 = 0x4d44;
const USER_ATTRIBUTES: &[u8] = b"user.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Created(PathBuf),
    Moved { from: PathBuf, to: PathBuf },
    Trashed(TrashItem),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Task {
    Copy {
        sources: Vec<PathBuf>,
        destination: PathBuf,
    },
    Move {
        sources: Vec<PathBuf>,
        destination: PathBuf,
    },
    Duplicate {
        sources: Vec<PathBuf>,
    },
    Trash {
        sources: Vec<PathBuf>,
    },
    Delete {
        sources: Vec<PathBuf>,
    },
    Restore {
        items: Vec<TrashItem>,
    },
    Erase {
        items: Vec<TrashItem>,
    },
    EmptyTrash,
    Revert {
        changes: Vec<Change>,
    },
    Compress {
        sources: Vec<PathBuf>,
        archive: PathBuf,
    },
    Extract {
        archives: Vec<PathBuf>,
        into: Option<PathBuf>,
        password: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    KeepBoth,
    Replace,
    Skip,
    Retry,
    Delete,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Answer {
    pub resolution: Resolution,
    pub apply_to_all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub source: PathBuf,
    pub target: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    pub items_done: u64,
    pub items_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current: Option<PathBuf>,
}

#[derive(Debug)]
pub struct Failure {
    pub path: PathBuf,
    pub error: io::Error,
    pub no_trash: bool,
}

const KERNEL_RESTART_CODES: std::ops::RangeInclusive<i32> = 512..=516;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    Permission,
    ReadOnly,
    NoSpace,
    NameTooLong,
    NameInvalid,
    TooLarge,
    DeviceGone,
    ServerGone,
    DiskError,
    Missing,
    Other,
}

impl Failure {
    pub fn problem(&self) -> Problem {
        let errno = |code: Errno| self.error.raw_os_error() == Some(code.raw_os_error());
        if errno(Errno::ACCESS) || errno(Errno::PERM) {
            Problem::Permission
        } else if [
            Errno::TIMEDOUT,
            Errno::HOSTDOWN,
            Errno::HOSTUNREACH,
            Errno::NETDOWN,
            Errno::NETUNREACH,
            Errno::NOTCONN,
            Errno::CONNRESET,
            Errno::CONNABORTED,
            Errno::STALE,
            Errno::AGAIN,
        ]
        .into_iter()
        .any(errno)
            || self
                .error
                .raw_os_error()
                .is_some_and(|code| KERNEL_RESTART_CODES.contains(&code))
        {
            Problem::ServerGone
        } else if errno(Errno::ROFS) {
            Problem::ReadOnly
        } else if errno(Errno::NOSPC) || errno(Errno::DQUOT) {
            Problem::NoSpace
        } else if errno(Errno::NAMETOOLONG) {
            Problem::NameTooLong
        } else if errno(Errno::INVAL) || errno(Errno::ILSEQ) {
            Problem::NameInvalid
        } else if errno(Errno::FBIG) {
            Problem::TooLarge
        } else if errno(Errno::NODEV) || errno(Errno::NXIO) || errno(Errno::SHUTDOWN) {
            Problem::DeviceGone
        } else if errno(Errno::IO) && bande_volumes::on_connected_volume(&self.path) {
            Problem::ServerGone
        } else if errno(Errno::IO) {
            Problem::DiskError
        } else if errno(Errno::NOENT) {
            Problem::Missing
        } else {
            Problem::Other
        }
    }

    pub fn retryable(&self) -> bool {
        !matches!(self.problem(), Problem::NameInvalid | Problem::TooLarge)
            && !matches!(
                self.error.kind(),
                io::ErrorKind::InvalidInput
                    | io::ErrorKind::AlreadyExists
                    | io::ErrorKind::Unsupported
            )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortfall {
    pub needed: u64,
    pub destination: PathBuf,
    pub sources: Vec<PathBuf>,
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub changes: Vec<Change>,
    pub failures: Vec<Failure>,
    pub shortfall: Option<Shortfall>,
    pub cancelled: bool,
}

#[derive(Debug)]
pub enum Report {
    Progress(Progress),
    Conflict(Conflict),
    Failure(Failure),
    Finished(Outcome),
}

#[derive(Debug)]
pub struct Job {
    reports: Receiver<Report>,
    answers: Sender<Answer>,
    cancelled: Arc<AtomicBool>,
    wake: UnixStream,
}

impl Job {
    pub fn start(task: Task, trash: Trash, asks: bool) -> io::Result<Self> {
        let (wake, signal) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        signal.set_nonblocking(true)?;
        let (report_sender, reports) = mpsc::channel();
        let (answers, answer_receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut worker = Worker {
            reports: report_sender,
            answers: answer_receiver,
            cancelled: cancelled.clone(),
            signal,
            trash,
            progress: Progress::default(),
            last_progress: None,
            remembered: None,
            outcome: Outcome::default(),
            stage: None,
            unflushed: 0,
            asks,
            skip_failures: false,
            delete_without_trash: false,
            skipped: 0,
        };
        thread::Builder::new()
            .name("file operation".to_owned())
            .spawn(move || worker.run(task))?;
        Ok(Self {
            reports,
            answers,
            cancelled,
            wake,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    pub fn take_reports(&self) -> Vec<Report> {
        let mut drained = [0u8; 256];
        while matches!((&self.wake).read(&mut drained), Ok(read) if read > 0) {}
        self.reports.try_iter().collect()
    }

    pub fn wait(&self) -> Report {
        loop {
            match self.reports.recv() {
                Ok(Report::Progress(_)) => {}
                Ok(report) => return report,
                Err(_) => return Report::Finished(Outcome::default()),
            }
        }
    }

    pub fn answer(&self, answer: Answer) {
        let _ = self.answers.send(answer);
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        let _ = (&self.wake).write(&[1]);
        self.answer(Answer {
            resolution: Resolution::Stop,
            apply_to_all: true,
        });
    }
}

enum Halt {
    Cancelled,
    Stopped,
}

struct Worker {
    reports: Sender<Report>,
    answers: Receiver<Answer>,
    cancelled: Arc<AtomicBool>,
    signal: UnixStream,
    trash: Trash,
    progress: Progress,
    last_progress: Option<Instant>,
    remembered: Option<Resolution>,
    outcome: Outcome,
    stage: Option<Stage>,
    unflushed: u64,
    asks: bool,
    skip_failures: bool,
    delete_without_trash: bool,
    skipped: usize,
}

struct Stage {
    path: PathBuf,
    anchor: Option<Anchor>,
}

struct Landing {
    target: PathBuf,
    replaces: bool,
}

impl Landing {
    fn free(target: PathBuf) -> Self {
        Self {
            target,
            replaces: false,
        }
    }
}

fn name_of(path: &Path) -> io::Result<&OsStr> {
    path.file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the item has no name"))
}

fn tree_size(path: &Path) -> (u64, u64) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return (0, 0);
    };
    if !metadata.is_dir() {
        return (1, metadata.len());
    }
    fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| tree_size(&entry.path()))
        .fold((1, 0), |(items, bytes), (more_items, more_bytes)| {
            (items + more_items, bytes + more_bytes)
        })
}

fn free_space(folder: &Path) -> Option<u64> {
    rustix::fs::statvfs(folder)
        .ok()
        .map(|stats| stats.f_bavail * stats.f_frsize)
}

fn keep_metadata(file: &File, metadata: &Metadata) -> io::Result<()> {
    file.set_permissions(metadata.permissions())?;
    let mut times = FileTimes::new().set_modified(metadata.modified()?);
    if let Ok(accessed) = metadata.accessed() {
        times = times.set_accessed(accessed);
    }
    file.set_times(times)
}

fn copy_attributes(from: &File, to: &File) {
    let mut names = vec![0; 64 * 1024];
    let Ok(listed) = flistxattr(from, &mut names) else {
        return;
    };
    let mut value = vec![0; 64 * 1024];
    for name in names[..listed].split(|byte| *byte == 0) {
        if name.starts_with(USER_ATTRIBUTES)
            && let name = OsStr::from_bytes(name)
            && let Ok(length) = fgetxattr(from, name, &mut value)
        {
            let _ = fsetxattr(to, name, &value[..length], XattrFlags::empty());
        }
    }
}

fn beyond_format_limit(output: &File, length: u64) -> bool {
    length > FAT_FILE_LIMIT && fstatfs(output).is_ok_and(|stats| stats.f_type == FAT_MAGIC.into())
}

fn crosses_devices(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::CrossesDevices
}

fn crosses_volumes(from: &Path, to: &Path) -> bool {
    let folder = to.parent().unwrap_or(Path::new("/"));
    match (fs::symlink_metadata(from), fs::metadata(folder)) {
        (Ok(from), Ok(folder)) => from.dev() != folder.dev(),
        _ => false,
    }
}

fn same_item(a: &Path, b: &Path) -> bool {
    match (fs::symlink_metadata(a), fs::symlink_metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

fn folder_contains(folder: &Path, inner: &Path) -> bool {
    if !fs::symlink_metadata(folder).is_ok_and(|metadata| metadata.is_dir()) {
        return false;
    }
    match (fs::canonicalize(folder), fs::canonicalize(inner)) {
        (Ok(folder), Ok(inner)) => inner.starts_with(folder),
        _ => inner.starts_with(folder),
    }
}

fn link_refused(error: io::Error) -> io::Error {
    match error.raw_os_error().map(Errno::from_raw_os_error) {
        Some(Errno::PERM | Errno::NOTSUP) => {
            io::Error::new(io::ErrorKind::Unsupported, L10N.tr("no-links"))
        }
        _ => error,
    }
}

fn taken(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::AlreadyExists
}

pub(crate) fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    match renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE) {
        Err(Errno::INVAL) if to.symlink_metadata().is_ok() => Err(io::Error::from(Errno::EXIST)),
        Err(Errno::INVAL) => fs::rename(from, to),
        result => result.map_err(io::Error::from),
    }
}

pub(crate) fn claim<T>(
    mut pick: impl FnMut() -> PathBuf,
    mut create: impl FnMut(&Path) -> io::Result<T>,
) -> io::Result<(PathBuf, T)> {
    loop {
        let path = pick();
        match create(&path) {
            Err(error) if taken(&error) => {}
            result => return result.map(|made| (path, made)),
        }
    }
}

impl Worker {
    fn send(&mut self, report: Report) {
        if self.reports.send(report).is_ok() {
            let _ = (&self.signal).write(&[1]);
        }
    }

    fn check(&self) -> Result<(), Halt> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err(Halt::Cancelled)
        } else {
            Ok(())
        }
    }

    fn report_progress(&mut self, force: bool) {
        let now = Instant::now();
        let due = self
            .last_progress
            .is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL);
        if force || due {
            self.last_progress = Some(now);
            self.send(Report::Progress(self.progress.clone()));
        }
    }

    fn fail(&mut self, path: &Path, error: io::Error) {
        self.skipped += 1;
        self.outcome.failures.push(Failure {
            path: path.to_path_buf(),
            error,
            no_trash: false,
        });
    }

    fn ask_about(&mut self, mut failure: Failure) -> Result<Answer, Halt> {
        let staged_on_server = self
            .stage
            .as_ref()
            .is_some_and(|stage| bande_volumes::on_connected_volume(&stage.path));
        if staged_on_server && failure.error.raw_os_error() == Some(Errno::IO.raw_os_error()) {
            failure.error = io::Error::from(Errno::HOSTDOWN);
        }
        self.send(Report::Failure(failure));
        let answer = self.answers.recv().map_err(|_| Halt::Cancelled)?;
        self.check()?;
        Ok(answer)
    }

    fn trouble(&mut self, path: &Path, error: io::Error) -> Result<bool, Halt> {
        if !self.asks {
            self.fail(path, error);
            return Ok(false);
        }
        if self.skip_failures {
            self.skipped += 1;
            return Ok(false);
        }
        let answer = self.ask_about(Failure {
            path: path.to_path_buf(),
            error,
            no_trash: false,
        })?;
        match answer.resolution {
            Resolution::Retry => Ok(true),
            Resolution::Stop => Err(Halt::Stopped),
            _ => {
                self.skipped += 1;
                self.skip_failures = answer.apply_to_all;
                Ok(false)
            }
        }
    }

    fn attempt<T>(
        &mut self,
        path: &Path,
        mut action: impl FnMut(&mut Self) -> io::Result<T>,
    ) -> Result<Option<T>, Halt> {
        loop {
            match action(self) {
                Ok(value) => return Ok(Some(value)),
                Err(error) => {
                    if !self.trouble(path, error)? {
                        return Ok(None);
                    }
                }
            }
        }
    }

    fn ensure_space(
        &mut self,
        destination: &Path,
        bytes: u64,
        sources: &[PathBuf],
    ) -> Result<(), Halt> {
        match free_space(destination) {
            Some(free) if bytes > free => {
                self.outcome.shortfall = Some(Shortfall {
                    needed: bytes - free,
                    destination: destination.to_path_buf(),
                    sources: sources.to_vec(),
                });
                Err(Halt::Stopped)
            }
            _ => Ok(()),
        }
    }

    fn ask(&mut self, conflict: Conflict) -> Result<Resolution, Halt> {
        if let Some(resolution) = self.remembered {
            return Ok(resolution);
        }
        self.send(Report::Conflict(conflict));
        let answer = self.answers.recv().map_err(|_| Halt::Cancelled)?;
        self.check()?;
        if answer.apply_to_all {
            self.remembered = Some(answer.resolution);
        }
        Ok(answer.resolution)
    }

    fn run(&mut self, task: Task) {
        let halted = self.perform(task);
        let mut outcome = std::mem::take(&mut self.outcome);
        outcome.cancelled = matches!(halted, Err(Halt::Cancelled));
        self.report_progress(true);
        self.send(Report::Finished(outcome));
    }

    fn count(&mut self, sources: &[PathBuf]) {
        for source in sources {
            let (items, bytes) = tree_size(source);
            self.progress.items_total += items;
            self.progress.bytes_total += bytes;
        }
        self.report_progress(true);
    }

    fn perform(&mut self, task: Task) -> Result<(), Halt> {
        match task {
            Task::Copy {
                sources,
                destination,
            } => {
                self.count(&sources);
                self.ensure_space(&destination, self.progress.bytes_total, &sources)?;
                for source in &sources {
                    self.copy_into(source, &destination)?;
                }
            }
            Task::Move {
                sources,
                destination,
            } => {
                self.progress.items_total = sources.len() as u64;
                let crossing = sources
                    .iter()
                    .filter(|source| !same_volume(source, &destination))
                    .map(|source| tree_size(source).1)
                    .sum();
                self.ensure_space(&destination, crossing, &sources)?;
                for source in &sources {
                    self.move_into(source, &destination)?;
                }
            }
            Task::Duplicate { sources } => {
                self.count(&sources);
                if let Some(folder) = sources.first().and_then(|source| source.parent()) {
                    self.ensure_space(folder, self.progress.bytes_total, &sources)?;
                }
                for source in &sources {
                    self.duplicate(source)?;
                }
            }
            Task::Trash { sources } => {
                self.progress.items_total = sources.len() as u64;
                for source in &sources {
                    self.check()?;
                    self.put_in_trash(source)?;
                    self.step(source);
                }
            }
            Task::Delete { sources } => {
                self.progress.items_total = sources.len() as u64;
                for source in &sources {
                    self.check()?;
                    self.attempt(source, |_| remove_all(source))?;
                    self.step(source);
                }
            }
            Task::Restore { items } => {
                self.progress.items_total = items.len() as u64;
                for item in &items {
                    self.restore(item)?;
                    self.step(&item.original);
                }
            }
            Task::Erase { items } => self.erase_all(&items)?,
            Task::EmptyTrash => {
                let items = self.trash.items();
                self.erase_all(&items)?;
            }
            Task::Compress { sources, archive } => self.compress(&sources, &archive)?,
            Task::Extract {
                archives,
                into,
                password,
            } => {
                for archive in &archives {
                    let folder = into
                        .as_deref()
                        .unwrap_or_else(|| archive.parent().unwrap_or(Path::new("/")));
                    self.extract(archive, folder, password.as_deref())?;
                }
            }
            Task::Revert { changes } => {
                self.progress.items_total = changes.len() as u64;
                for change in changes.iter().rev() {
                    self.check()?;
                    self.revert(change)?;
                }
            }
        }
        Ok(())
    }

    fn step(&mut self, current: &Path) {
        self.progress.items_done += 1;
        self.progress.current = Some(current.to_path_buf());
        self.report_progress(false);
    }

    fn erase_all(&mut self, items: &[TrashItem]) -> Result<(), Halt> {
        self.progress.items_total = items.len() as u64;
        for item in items {
            self.check()?;
            self.attempt(&item.original, |_| erase(item))?;
            self.step(&item.original);
        }
        Ok(())
    }

    fn delete_instead(&mut self, path: &Path) -> Result<bool, Halt> {
        Ok(self.attempt(path, |_| remove_all(path))?.is_some())
    }

    fn put_in_trash(&mut self, path: &Path) -> Result<bool, Halt> {
        loop {
            let error = match self.trash.put(path) {
                Ok(item) => {
                    self.outcome.changes.push(Change::Trashed(item));
                    return Ok(true);
                }
                Err(error) => error,
            };
            let no_trash = self.asks
                && error.raw_os_error() != Some(Errno::ROFS.raw_os_error())
                && path.symlink_metadata().is_ok()
                && (crosses_devices(&error) || !self.trash.available_for(path));
            if !no_trash {
                return match self.trouble(path, error)? {
                    true => continue,
                    false => Ok(false),
                };
            }
            if self.delete_without_trash {
                return self.delete_instead(path);
            }
            if self.skip_failures {
                self.skipped += 1;
                return Ok(false);
            }
            let answer = self.ask_about(Failure {
                path: path.to_path_buf(),
                error,
                no_trash,
            })?;
            match answer.resolution {
                Resolution::Delete => {
                    self.delete_without_trash = answer.apply_to_all;
                    return self.delete_instead(path);
                }
                Resolution::Retry => {}
                Resolution::Stop => return Err(Halt::Stopped),
                _ => {
                    self.skipped += 1;
                    self.skip_failures = answer.apply_to_all;
                    return Ok(false);
                }
            }
        }
    }

    fn resolve(&mut self, source: &Path, target: PathBuf) -> Result<Option<Landing>, Halt> {
        if target.symlink_metadata().is_err() {
            return Ok(Some(Landing::free(target)));
        }
        let resolution = self.ask(Conflict {
            source: source.to_path_buf(),
            target: target.clone(),
        })?;
        match resolution {
            Resolution::KeepBoth => {
                let folder = target.parent().unwrap_or(Path::new("/"));
                let name = target.file_name().unwrap_or_default();
                Ok(Some(Landing::free(keep_both_name(
                    folder,
                    name,
                    source.is_dir(),
                ))))
            }
            Resolution::Replace => {
                if source.starts_with(&target) {
                    self.trouble(
                        source,
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "an item can't replace the folder that contains it",
                        ),
                    )?;
                    return Ok(None);
                }
                Ok(Some(Landing {
                    target,
                    replaces: true,
                }))
            }
            Resolution::Skip | Resolution::Retry | Resolution::Delete => Ok(None),
            Resolution::Stop => Err(Halt::Stopped),
        }
    }

    fn copy_into(&mut self, source: &Path, destination: &Path) -> Result<(), Halt> {
        self.check()?;
        let name = match name_of(source) {
            Ok(name) => name.to_os_string(),
            Err(error) => {
                self.trouble(source, error)?;
                return Ok(());
            }
        };
        if folder_contains(source, destination) {
            self.trouble(
                source,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a folder can't be copied into itself",
                ),
            )?;
            return Ok(());
        }
        let target = destination.join(&name);
        if same_item(source, &target) {
            return self.copy_duplicate(source, destination, &name);
        }
        match self.resolve(source, target)? {
            Some(landing) => self.copy_item(source, landing, |worker, target| {
                worker.resolve(source, target)
            }),
            None => Ok(()),
        }
    }

    fn copy_duplicate(
        &mut self,
        source: &Path,
        folder: &Path,
        name: &OsString,
    ) -> Result<(), Halt> {
        let is_dir = source.is_dir();
        let landing = Landing::free(duplicate_name(folder, name, is_dir));
        self.copy_item(source, landing, |_, _| {
            Ok(Some(Landing::free(duplicate_name(folder, name, is_dir))))
        })
    }

    fn discard_stage(&mut self) {
        if let Some(stage) = self.stage.take() {
            let _ = remove_all(&stage.path);
        }
    }

    fn anchor_stage(&mut self, target: &Path, anchor: impl FnOnce() -> io::Result<Anchor>) {
        if let Some(stage) = &mut self.stage
            && stage.path == target
        {
            stage.anchor = anchor().ok();
        }
    }

    fn flush_stage(&mut self, written: u64) -> io::Result<()> {
        self.unflushed += written;
        if self.unflushed < FLUSH_EVERY {
            return Ok(());
        }
        self.unflushed = 0;
        match self.stage.as_ref().and_then(|stage| stage.anchor.as_ref()) {
            Some(anchor) => anchor.flush(),
            None => Ok(()),
        }
    }

    fn stage_copy(&mut self, source: &Path, folder: &Path, whole: bool) -> Result<bool, Halt> {
        let (items_done, bytes_done) = (self.progress.items_done, self.progress.bytes_done);
        loop {
            let path = partial_path(folder);
            self.stage = Some(Stage {
                path: path.clone(),
                anchor: None,
            });
            self.unflushed = 0;
            let skipped = self.skipped;
            let placed = match self.copy_tree(source, &path) {
                Ok(placed) => placed,
                Err(halt) => {
                    self.discard_stage();
                    return Err(halt);
                }
            };
            if !placed || (whole && self.skipped > skipped) {
                self.discard_stage();
                return Ok(false);
            }
            let settled = self
                .stage
                .as_ref()
                .and_then(|stage| stage.anchor.as_ref())
                .map_or(Ok(()), Anchor::settle);
            let Err(error) = settled else {
                return Ok(true);
            };
            self.discard_stage();
            self.progress.items_done = items_done;
            self.progress.bytes_done = bytes_done;
            if !self.trouble(source, error)? {
                return Ok(false);
            }
        }
    }

    fn land(
        &mut self,
        landing: Landing,
        elsewhere: impl FnMut(&mut Self, PathBuf) -> Result<Option<Landing>, Halt>,
    ) -> Result<Option<(PathBuf, io::Result<()>)>, Halt> {
        let landed = self.land_stage(landing, elsewhere);
        if matches!(landed, Ok(Some(_))) {
            self.stage = None;
        } else {
            self.discard_stage();
        }
        landed
    }

    fn land_stage(
        &mut self,
        mut landing: Landing,
        mut elsewhere: impl FnMut(&mut Self, PathBuf) -> Result<Option<Landing>, Halt>,
    ) -> Result<Option<(PathBuf, io::Result<()>)>, Halt> {
        let Some(staged) = self.stage.as_ref().map(|stage| stage.path.clone()) else {
            return Ok(None);
        };
        loop {
            self.check()?;
            if landing.replaces && !self.put_in_trash(&landing.target)? {
                return Ok(None);
            }
            landing.replaces = false;
            match rename_new(&staged, &landing.target) {
                Ok(()) => {
                    let folder = landing.target.parent().unwrap_or(Path::new("/"));
                    let settled = sync_folder(folder);
                    return Ok(Some((landing.target, settled)));
                }
                Err(error) if taken(&error) => match elsewhere(self, landing.target)? {
                    Some(next) => landing = next,
                    None => return Ok(None),
                },
                Err(error) => {
                    if !self.trouble(&landing.target, error)? {
                        return Ok(None);
                    }
                }
            }
        }
    }

    fn copy_item(
        &mut self,
        source: &Path,
        landing: Landing,
        elsewhere: impl FnMut(&mut Self, PathBuf) -> Result<Option<Landing>, Halt>,
    ) -> Result<(), Halt> {
        let folder = landing.target.parent().unwrap_or(Path::new("/"));
        if !self.stage_copy(source, folder, false)? {
            return Ok(());
        }
        if let Some((target, settled)) = self.land(landing, elsewhere)? {
            self.outcome.changes.push(Change::Created(target.clone()));
            if let Err(error) = settled {
                self.trouble(&target, error)?;
            }
        }
        Ok(())
    }

    fn duplicate(&mut self, source: &Path) -> Result<(), Halt> {
        self.check()?;
        let (Some(folder), Ok(name)) = (source.parent(), name_of(source)) else {
            self.trouble(
                source,
                io::Error::new(io::ErrorKind::InvalidInput, "the item has no folder"),
            )?;
            return Ok(());
        };
        self.copy_duplicate(source, folder, &name.to_os_string())
    }

    fn copy_tree(&mut self, source: &Path, target: &Path) -> Result<bool, Halt> {
        self.check()?;
        self.progress.current = Some(source.to_path_buf());
        let mut placed = true;
        while let Err(error) = self.copy_entry(source, target)? {
            if !self.trouble(source, error)? {
                placed = false;
                break;
            }
        }
        self.progress.items_done += 1;
        self.report_progress(false);
        Ok(placed)
    }

    fn copy_entry(&mut self, source: &Path, target: &Path) -> Result<io::Result<()>, Halt> {
        let metadata = match fs::symlink_metadata(source) {
            Ok(metadata) => metadata,
            Err(error) => return Ok(Err(error)),
        };
        let file_type = metadata.file_type();
        Ok(if file_type.is_symlink() {
            fs::read_link(source).and_then(|link| symlink(link, target).map_err(link_refused))
        } else if file_type.is_dir() {
            self.copy_folder(source, target, &metadata)?
        } else if file_type.is_file() {
            self.copy_file(source, target, &metadata)?
        } else {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "special files can't be copied",
            ))
        })
    }

    fn copy_folder(
        &mut self,
        source: &Path,
        target: &Path,
        metadata: &Metadata,
    ) -> Result<io::Result<()>, Halt> {
        let listed = fs::read_dir(source).and_then(|entries| {
            entries
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect::<io::Result<Vec<_>>>()
        });
        let mut children = match listed {
            Ok(children) => children,
            Err(error) => return Ok(Err(error)),
        };
        if let Err(error) = fs::create_dir(target) {
            return Ok(Err(error));
        }
        self.anchor_stage(target, || Anchor::folder(target));
        children.sort();
        for child in children {
            self.copy_tree(&source.join(&child), &target.join(&child))?;
        }
        if let Ok(folder) = File::open(target) {
            if let Ok(original) = File::open(source) {
                copy_attributes(&original, &folder);
            }
            let _ = keep_metadata(&folder, metadata);
        }
        Ok(Ok(()))
    }

    fn copy_file(
        &mut self,
        source: &Path,
        target: &Path,
        metadata: &Metadata,
    ) -> Result<io::Result<()>, Halt> {
        if self
            .stage
            .as_ref()
            .is_some_and(|stage| target.starts_with(&stage.path))
        {
            let _ = fs::remove_file(target);
        }
        let opened = File::open(source).and_then(|input| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(target)
                .map(|output| (input, output))
        });
        let (input, mut output) = match opened {
            Ok(files) => files,
            Err(error) => return Ok(Err(error)),
        };
        if beyond_format_limit(&output, metadata.len()) {
            drop(output);
            let _ = fs::remove_file(target);
            return Ok(Err(io::Error::from(Errno::FBIG)));
        }
        self.anchor_stage(target, || Anchor::file(&output));
        let mut written = 0;
        loop {
            if self.check().is_err() {
                drop(output);
                let _ = fs::remove_file(target);
                return Err(Halt::Cancelled);
            }
            let chunk = io::copy(&mut (&input).take(CHUNK), &mut output);
            let chunk = chunk.and_then(|copied| self.flush_stage(copied).map(|()| copied));
            match chunk {
                Ok(0) => break,
                Ok(copied) => {
                    written += copied;
                    self.progress.bytes_done += copied;
                    self.report_progress(false);
                }
                Err(error) => {
                    self.progress.bytes_done -= written;
                    drop(output);
                    let _ = fs::remove_file(target);
                    return Ok(Err(error));
                }
            }
        }
        copy_attributes(&input, &output);
        let _ = keep_metadata(&output, metadata);
        Ok(Ok(()))
    }

    fn relocate(&mut self, from: &Path, mut landing: Landing) -> Result<Option<Change>, Halt> {
        if crosses_volumes(from, &landing.target) {
            return self.copy_across(from, landing);
        }
        loop {
            if landing.replaces && !self.put_in_trash(&landing.target)? {
                return Ok(None);
            }
            landing.replaces = false;
            match rename_new(from, &landing.target) {
                Ok(()) => {
                    return Ok(Some(Change::Moved {
                        from: from.to_path_buf(),
                        to: landing.target,
                    }));
                }
                Err(error) if crosses_devices(&error) => return self.copy_across(from, landing),
                Err(error) if taken(&error) => match self.resolve(from, landing.target)? {
                    Some(next) => landing = next,
                    None => return Ok(None),
                },
                Err(error) => {
                    if !self.trouble(from, error)? {
                        return Ok(None);
                    }
                }
            }
        }
    }

    fn copy_across(&mut self, from: &Path, landing: Landing) -> Result<Option<Change>, Halt> {
        let (items, bytes) = tree_size(from);
        self.progress.items_total += items;
        self.progress.bytes_total += bytes;
        let folder = landing.target.parent().unwrap_or(Path::new("/"));
        if !self.stage_copy(from, folder, true)? {
            return Ok(None);
        }
        let landed = self.land(landing, |worker, target| worker.resolve(from, target))?;
        let Some((to, settled)) = landed else {
            return Ok(None);
        };
        if let Err(error) = settled {
            self.trouble(from, error)?;
            return Ok(Some(Change::Created(to)));
        }
        Ok(Some(match self.attempt(from, |_| remove_all(from))? {
            Some(()) => Change::Moved {
                from: from.to_path_buf(),
                to,
            },
            None => Change::Created(to),
        }))
    }

    fn move_into(&mut self, source: &Path, destination: &Path) -> Result<(), Halt> {
        self.check()?;
        let name = match name_of(source) {
            Ok(name) => name.to_owned(),
            Err(error) => {
                self.trouble(source, error)?;
                return Ok(());
            }
        };
        let target = destination.join(&name);
        if same_item(source, &target) {
            self.step(source);
            return Ok(());
        }
        if folder_contains(source, destination) {
            self.trouble(
                source,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a folder can't be moved into itself",
                ),
            )?;
            return Ok(());
        }
        if let Some(landing) = self.resolve(source, target)?
            && let Some(change) = self.relocate(source, landing)?
        {
            self.outcome.changes.push(change);
        }
        self.step(source);
        Ok(())
    }

    fn restore(&mut self, item: &TrashItem) -> Result<(), Halt> {
        self.check()?;
        let Some(landing) = self.resolve(&item.path(), item.original.clone())? else {
            return Ok(());
        };
        self.put_back(item, landing)
    }

    fn put_back(&mut self, item: &TrashItem, mut landing: Landing) -> Result<(), Halt> {
        let placed = loop {
            if crosses_volumes(&item.path(), &landing.target) {
                break self.put_back_across(item, landing)?;
            }
            if landing.replaces && !self.put_in_trash(&landing.target)? {
                return Ok(());
            }
            landing.replaces = false;
            match restore_to(item, &landing.target) {
                Ok(()) => break Some(landing.target),
                Err(error) if crosses_devices(&error) => {
                    break self.put_back_across(item, landing)?;
                }
                Err(error) if taken(&error) => match self.resolve(&item.path(), landing.target)? {
                    Some(next) => landing = next,
                    None => return Ok(()),
                },
                Err(error) => {
                    if !self.trouble(&landing.target, error)? {
                        return Ok(());
                    }
                }
            }
        };
        if let Some(placed) = placed {
            self.outcome.changes.push(Change::Created(placed));
        }
        Ok(())
    }

    fn put_back_across(
        &mut self,
        item: &TrashItem,
        landing: Landing,
    ) -> Result<Option<PathBuf>, Halt> {
        Ok(match self.copy_across(&item.path(), landing)? {
            Some(Change::Moved { to, .. }) => {
                if let Err(error) = forget(item) {
                    self.fail(&to, error);
                }
                Some(to)
            }
            Some(Change::Created(to)) => Some(to),
            _ => None,
        })
    }

    fn revert(&mut self, change: &Change) -> Result<(), Halt> {
        match change {
            Change::Created(path) => {
                self.put_in_trash(path)?;
                self.step(path);
            }
            Change::Moved { from, to } => {
                if from.symlink_metadata().is_ok() {
                    self.trouble(from, io::Error::from(Errno::EXIST))?;
                } else if let Some(back) = self.relocate(to, Landing::free(from.clone()))? {
                    self.outcome.changes.push(back);
                }
                self.step(from);
            }
            Change::Trashed(item) => {
                self.put_back(item, Landing::free(item.original.clone()))?;
                self.step(&item.original);
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum RenameError {
    Empty,
    ContainsSlash,
    Reserved,
    Exists,
    Io(io::Error),
}

pub fn rename(path: &Path, new_name: &str) -> Result<Option<Change>, RenameError> {
    if new_name.is_empty() {
        return Err(RenameError::Empty);
    }
    if new_name.contains('/') {
        return Err(RenameError::ContainsSlash);
    }
    if new_name == "." || new_name == ".." {
        return Err(RenameError::Reserved);
    }
    let target = path.with_file_name(new_name);
    let unchanged = path
        .file_name()
        .is_some_and(|name| name.to_string_lossy() == new_name);
    if target == path || unchanged {
        return Ok(None);
    }
    match rename_new(path, &target) {
        Ok(()) => {}
        Err(error) if taken(&error) && same_item(path, &target) => {
            fs::rename(path, &target).map_err(RenameError::Io)?
        }
        Err(error) if taken(&error) => return Err(RenameError::Exists),
        Err(error) => return Err(RenameError::Io(error)),
    }
    Ok(Some(Change::Moved {
        from: path.to_path_buf(),
        to: target,
    }))
}

pub fn make_alias(path: &Path) -> io::Result<Change> {
    let folder = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let name = name_of(path)?;
    let (alias, ()) = claim(|| alias_name(folder, name), |alias| symlink(path, alias))?;
    Ok(Change::Created(alias))
}

pub fn new_document(parent: &Path, template: &Path) -> io::Result<Change> {
    let name = name_of(template)?;
    let mut first = Some(parent.join(name));
    let mut input = File::open(template)?;
    let permissions = input.metadata()?.permissions();
    let (path, ()) = claim(
        || {
            first
                .take()
                .unwrap_or_else(|| keep_both_name(parent, name, false))
        },
        |path| {
            let mut output = File::create_new(path)?;
            let written = io::copy(&mut input, &mut output)
                .and_then(|_| output.set_permissions(permissions.clone()));
            if written.is_err() {
                let _ = fs::remove_file(path);
            }
            written
        },
    )?;
    Ok(Change::Created(path))
}

pub fn new_folder(parent: &Path) -> io::Result<Change> {
    let (path, ()) = claim(|| untitled_folder(parent), |path| fs::create_dir(path))?;
    Ok(Change::Created(path))
}
