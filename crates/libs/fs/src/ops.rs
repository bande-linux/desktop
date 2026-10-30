use std::fs::{self, File, FileTimes, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::fs::{OpenOptionsExt, symlink};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

mod archive;

pub use archive::{archive_path, can_extract};

use crate::naming::{duplicate_name, keep_both_name, untitled_folder};
use crate::trash::{Trash, TrashItem, erase, forget, remove_all, restore_to};

const CHUNK: u64 = 8 * 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

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
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    KeepBoth,
    Replace,
    Skip,
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
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub changes: Vec<Change>,
    pub failures: Vec<Failure>,
    pub cancelled: bool,
}

#[derive(Debug)]
pub enum Report {
    Progress(Progress),
    Conflict(Conflict),
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
    pub fn start(task: Task, trash: Trash) -> io::Result<Self> {
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
}

fn name_of(path: &Path) -> io::Result<&std::ffi::OsStr> {
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

fn keep_metadata(file: &File, metadata: &Metadata) -> io::Result<()> {
    file.set_permissions(metadata.permissions())?;
    let mut times = FileTimes::new().set_modified(metadata.modified()?);
    if let Ok(accessed) = metadata.accessed() {
        times = times.set_accessed(accessed);
    }
    file.set_times(times)
}

fn crosses_devices(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::CrossesDevices
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
        self.outcome.failures.push(Failure {
            path: path.to_path_buf(),
            error,
        });
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
                for source in &sources {
                    self.copy_into(source, &destination)?;
                }
            }
            Task::Move {
                sources,
                destination,
            } => {
                self.progress.items_total = sources.len() as u64;
                for source in &sources {
                    self.move_into(source, &destination)?;
                }
            }
            Task::Duplicate { sources } => {
                self.count(&sources);
                for source in &sources {
                    self.duplicate(source)?;
                }
            }
            Task::Trash { sources } => {
                self.progress.items_total = sources.len() as u64;
                for source in &sources {
                    self.check()?;
                    self.put_in_trash(source);
                    self.step(source);
                }
            }
            Task::Delete { sources } => {
                self.progress.items_total = sources.len() as u64;
                for source in &sources {
                    self.check()?;
                    if let Err(error) = remove_all(source) {
                        self.fail(source, error);
                    }
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
            Task::Extract { archives } => {
                for archive in &archives {
                    let folder = archive.parent().unwrap_or(Path::new("/"));
                    self.extract(archive, folder)?;
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
            if let Err(error) = erase(item) {
                self.fail(&item.original, error);
            }
            self.step(&item.original);
        }
        Ok(())
    }

    fn put_in_trash(&mut self, path: &Path) -> Option<TrashItem> {
        match self.trash.put(path) {
            Ok(item) => {
                self.outcome.changes.push(Change::Trashed(item.clone()));
                Some(item)
            }
            Err(error) => {
                self.fail(path, error);
                None
            }
        }
    }

    fn resolve(&mut self, source: &Path, target: PathBuf) -> Result<Option<PathBuf>, Halt> {
        if target.symlink_metadata().is_err() {
            return Ok(Some(target));
        }
        let resolution = self.ask(Conflict {
            source: source.to_path_buf(),
            target: target.clone(),
        })?;
        match resolution {
            Resolution::KeepBoth => {
                let folder = target.parent().unwrap_or(Path::new("/"));
                let name = target
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Ok(Some(keep_both_name(folder, &name, source.is_dir())))
            }
            Resolution::Replace => {
                if source.starts_with(&target) {
                    self.fail(
                        source,
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "an item can't replace the folder that contains it",
                        ),
                    );
                    return Ok(None);
                }
                Ok(self.put_in_trash(&target).map(|_| target))
            }
            Resolution::Skip => Ok(None),
            Resolution::Stop => Err(Halt::Stopped),
        }
    }

    fn copy_into(&mut self, source: &Path, destination: &Path) -> Result<(), Halt> {
        self.check()?;
        let name = match name_of(source) {
            Ok(name) => name.to_owned(),
            Err(error) => {
                self.fail(source, error);
                return Ok(());
            }
        };
        if destination.starts_with(source) {
            self.fail(
                source,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a folder can't be copied into itself",
                ),
            );
            return Ok(());
        }
        let target = destination.join(&name);
        let target = if target == source {
            duplicate_name(destination, &name.to_string_lossy(), source.is_dir())
        } else {
            match self.resolve(source, target)? {
                Some(target) => target,
                None => return Ok(()),
            }
        };
        self.copy_item(source, &target)
    }

    fn copy_item(&mut self, source: &Path, target: &Path) -> Result<(), Halt> {
        match self.copy_tree(source, target) {
            Ok(()) => {
                if target.symlink_metadata().is_ok() {
                    self.outcome
                        .changes
                        .push(Change::Created(target.to_path_buf()));
                }
                Ok(())
            }
            Err(halt) => {
                let _ = remove_all(target);
                Err(halt)
            }
        }
    }

    fn duplicate(&mut self, source: &Path) -> Result<(), Halt> {
        self.check()?;
        let (Some(folder), Ok(name)) = (source.parent(), name_of(source)) else {
            self.fail(
                source,
                io::Error::new(io::ErrorKind::InvalidInput, "the item has no folder"),
            );
            return Ok(());
        };
        let target = duplicate_name(folder, &name.to_string_lossy(), source.is_dir());
        self.copy_item(source, &target)
    }

    fn copy_tree(&mut self, source: &Path, target: &Path) -> Result<(), Halt> {
        self.check()?;
        self.progress.current = Some(source.to_path_buf());
        let metadata = match fs::symlink_metadata(source) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.fail(source, error);
                return Ok(());
            }
        };
        let file_type = metadata.file_type();
        let copied = if file_type.is_symlink() {
            fs::read_link(source).and_then(|link| symlink(link, target))
        } else if file_type.is_dir() {
            self.copy_folder(source, target, &metadata)?
        } else if file_type.is_file() {
            self.copy_file(source, target, &metadata)?
        } else {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "special files can't be copied",
            ))
        };
        if let Err(error) = copied {
            self.fail(source, error);
        }
        self.progress.items_done += 1;
        self.report_progress(false);
        Ok(())
    }

    fn copy_folder(
        &mut self,
        source: &Path,
        target: &Path,
        metadata: &Metadata,
    ) -> Result<io::Result<()>, Halt> {
        if let Err(error) = fs::create_dir(target) {
            return Ok(Err(error));
        }
        let mut children = match fs::read_dir(source) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name())
                .collect::<Vec<_>>(),
            Err(error) => return Ok(Err(error)),
        };
        children.sort();
        for child in children {
            self.copy_tree(&source.join(&child), &target.join(&child))?;
        }
        Ok(File::open(target).and_then(|folder| keep_metadata(&folder, metadata)))
    }

    fn copy_file(
        &mut self,
        source: &Path,
        target: &Path,
        metadata: &Metadata,
    ) -> Result<io::Result<()>, Halt> {
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
        loop {
            if self.check().is_err() {
                drop(output);
                let _ = fs::remove_file(target);
                return Err(Halt::Cancelled);
            }
            match io::copy(&mut (&input).take(CHUNK), &mut output) {
                Ok(0) => break,
                Ok(copied) => {
                    self.progress.bytes_done += copied;
                    self.report_progress(false);
                }
                Err(error) => {
                    drop(output);
                    let _ = fs::remove_file(target);
                    return Ok(Err(error));
                }
            }
        }
        Ok(keep_metadata(&output, metadata))
    }

    fn relocate(&mut self, from: &Path, to: &Path) -> Result<bool, Halt> {
        match fs::rename(from, to) {
            Ok(()) => Ok(true),
            Err(error) if crosses_devices(&error) => {
                let (items, bytes) = tree_size(from);
                self.progress.items_total += items;
                self.progress.bytes_total += bytes;
                let failures = self.outcome.failures.len();
                if let Err(halt) = self.copy_tree(from, to) {
                    let _ = remove_all(to);
                    return Err(halt);
                }
                if self.outcome.failures.len() > failures {
                    let _ = remove_all(to);
                    return Ok(false);
                }
                if let Err(error) = remove_all(from) {
                    self.fail(from, error);
                }
                Ok(true)
            }
            Err(error) => {
                self.fail(from, error);
                Ok(false)
            }
        }
    }

    fn move_into(&mut self, source: &Path, destination: &Path) -> Result<(), Halt> {
        self.check()?;
        let name = match name_of(source) {
            Ok(name) => name.to_owned(),
            Err(error) => {
                self.fail(source, error);
                return Ok(());
            }
        };
        let target = destination.join(&name);
        if target == source {
            self.step(source);
            return Ok(());
        }
        if destination.starts_with(source) {
            self.fail(
                source,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a folder can't be moved into itself",
                ),
            );
            return Ok(());
        }
        if let Some(target) = self.resolve(source, target)?
            && self.relocate(source, &target)?
        {
            self.outcome.changes.push(Change::Moved {
                from: source.to_path_buf(),
                to: target,
            });
        }
        self.step(source);
        Ok(())
    }

    fn restore(&mut self, item: &TrashItem) -> Result<(), Halt> {
        self.check()?;
        let Some(target) = self.resolve(&item.path(), item.original.clone())? else {
            return Ok(());
        };
        self.put_back(item, &target)
    }

    fn put_back(&mut self, item: &TrashItem, target: &Path) -> Result<(), Halt> {
        match restore_to(item, target) {
            Ok(()) => {}
            Err(error) if crosses_devices(&error) => {
                if !self.relocate(&item.path(), target)? {
                    return Ok(());
                }
                if let Err(error) = forget(item) {
                    self.fail(target, error);
                }
            }
            Err(error) => {
                self.fail(target, error);
                return Ok(());
            }
        }
        self.outcome
            .changes
            .push(Change::Created(target.to_path_buf()));
        Ok(())
    }

    fn revert(&mut self, change: &Change) -> Result<(), Halt> {
        match change {
            Change::Created(path) => {
                self.put_in_trash(path);
                self.step(path);
            }
            Change::Moved { from, to } => {
                if from.symlink_metadata().is_ok() {
                    self.fail(
                        from,
                        io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            "an item with that name already exists",
                        ),
                    );
                } else if self.relocate(to, from)? {
                    self.outcome.changes.push(Change::Moved {
                        from: to.clone(),
                        to: from.clone(),
                    });
                }
                self.step(from);
            }
            Change::Trashed(item) => {
                self.put_back(item, &item.original)?;
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
    if target == path {
        return Ok(None);
    }
    let same_item = |a: &Path, b: &Path| {
        use std::os::unix::fs::MetadataExt;
        match (fs::symlink_metadata(a), fs::symlink_metadata(b)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
            _ => false,
        }
    };
    if target.symlink_metadata().is_ok() && !same_item(path, &target) {
        return Err(RenameError::Exists);
    }
    fs::rename(path, &target).map_err(RenameError::Io)?;
    Ok(Some(Change::Moved {
        from: path.to_path_buf(),
        to: target,
    }))
}

pub fn new_folder(parent: &Path) -> io::Result<Change> {
    let path = untitled_folder(parent);
    fs::create_dir(&path)?;
    Ok(Change::Created(path))
}
