use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread;

use bande_inbox::{Inbox, Outbox};
use bande_watch::{Changes, FolderWatch};

use crate::Found;
use crate::matcher::Query;
use crate::tree::{ROOT, Tree};

pub const MAX_HITS: usize = 40;
pub const MAX_ENTRIES: usize = 400_000;
const FOLDERS_PER_BATCH: usize = 64;
const SKIPPED_FOLDERS: &[&str] = &["node_modules", "__pycache__", "snap"];
pub const CACHE_TAG: &str = "CACHEDIR.TAG";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHit {
    pub path: PathBuf,
    pub folder: bool,
    pub score: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileResults {
    pub generation: u64,
    pub hits: Vec<FileHit>,
    pub items: Vec<Found>,
}

#[derive(Debug, Clone)]
struct Asked {
    generation: u64,
    text: String,
    within: Option<PathBuf>,
    limit: usize,
}

enum Command {
    Query(Asked),
    Changed(Vec<PathBuf>),
    Exclude(Vec<PathBuf>),
}

pub struct FileIndex {
    commands: Sender<Command>,
    results: Inbox<FileResults>,
}

impl FileIndex {
    pub fn spawn(root: PathBuf, excluded: Vec<PathBuf>) -> io::Result<Self> {
        let (commands, received) = channel();
        let results = Inbox::new()?;
        let sender = results.outbox();
        let changes = commands.clone();
        let watcher = FolderWatch::spawn(Changes::Any, move |paths| {
            let _ = changes.send(Command::Changed(paths));
        })
        .inspect_err(|error| tracing::warn!("cannot watch the home folder: {error}"))
        .ok();
        thread::Builder::new()
            .name("file-index".to_owned())
            .spawn(move || {
                let mut worker = Worker {
                    root,
                    excluded,
                    tree: Tree::rooted(),
                    watcher,
                    commands: received,
                    results: sender,
                    pending: None,
                    answered: None,
                };
                worker.run();
            })?;
        Ok(Self { commands, results })
    }

    pub fn query_within(&self, generation: u64, text: &str, within: &Path, limit: usize) {
        let _ = self.commands.send(Command::Query(Asked {
            generation,
            text: text.to_owned(),
            within: Some(within.to_path_buf()),
            limit,
        }));
    }

    pub fn exclude(&self, excluded: Vec<PathBuf>) {
        let _ = self.commands.send(Command::Exclude(excluded));
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.results.fd()
    }

    pub fn take(&mut self) -> Option<FileResults> {
        self.results.drain().pop()
    }
}

struct Worker {
    root: PathBuf,
    excluded: Vec<PathBuf>,
    tree: Tree,
    watcher: Option<FolderWatch>,
    commands: Receiver<Command>,
    results: Outbox<FileResults>,
    pending: Option<Asked>,
    answered: Option<Asked>,
}

pub fn skipped_name(name: &str) -> bool {
    bande_fs::is_hidden(name) || SKIPPED_FOLDERS.contains(&name)
}

fn file_name(path: &Path) -> Option<&str> {
    path.file_name()?.to_str()
}

impl Worker {
    fn run(&mut self) {
        let mut folders = vec![(ROOT, self.root.clone())];
        self.watch(&self.root.clone());
        let mut read = 0usize;
        while let Some((id, folder)) = folders.pop() {
            self.read_folder(id, &folder, &mut folders);
            read += 1;
            if read.is_multiple_of(FOLDERS_PER_BATCH) && !self.drain() {
                return;
            }
        }
        self.pending = self.pending.take().or(self.answered.take());
        if !self.drain() {
            return;
        }
        while let Ok(command) = self.commands.recv() {
            self.apply(command);
            if !self.drain() {
                return;
            }
        }
    }

    fn drain(&mut self) -> bool {
        loop {
            match self.commands.try_recv() {
                Ok(command) => self.apply(command),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
        if let Some(asked) = self.pending.take() {
            let hits = self.search(&asked);
            let generation = asked.generation;
            self.answered = Some(asked);
            let items = Vec::new();
            if !self.results.send(FileResults {
                generation,
                hits,
                items,
            }) {
                return false;
            }
        }
        true
    }

    fn apply(&mut self, command: Command) {
        match command {
            Command::Query(asked) => self.pending = Some(asked),
            Command::Changed(paths) => {
                for path in paths {
                    self.changed(&path);
                }
            }
            Command::Exclude(excluded) => {
                let previous = std::mem::replace(&mut self.excluded, excluded);
                for folder in self.excluded.clone() {
                    if !previous.contains(&folder) {
                        self.remove(&folder);
                    }
                }
                for folder in previous {
                    if !self.excluded.contains(&folder) {
                        self.changed(&folder);
                    }
                }
            }
        }
    }

    fn is_excluded(&self, path: &Path) -> bool {
        crate::is_excluded(path, &self.excluded)
    }

    fn indexed(&self, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(&self.root) else {
            return false;
        };
        !self.is_excluded(path)
            && relative
                .components()
                .all(|component| !component.as_os_str().to_str().is_none_or(skipped_name))
    }

    fn changed(&mut self, path: &Path) {
        if !self.indexed(path) || path == self.root {
            return;
        }
        if file_name(path) == Some(CACHE_TAG)
            && let Some(parent) = path.parent()
        {
            self.remove(parent);
            return;
        }
        let (Some(parent), Some(name)) = (
            path.parent()
                .and_then(|parent| self.tree.find(&self.root, parent)),
            file_name(path),
        ) else {
            return;
        };
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_dir() => {
                if self.tree.child(parent, name).is_none()
                    && let Some(id) = self.tree.add(parent, name, true)
                {
                    let mut folders = vec![(id, path.to_path_buf())];
                    while let Some((id, folder)) = folders.pop() {
                        self.read_folder(id, &folder, &mut folders);
                    }
                }
            }
            Ok(metadata) if metadata.is_file() => {
                self.tree.insert(parent, name, false);
            }
            Ok(_) => {}
            Err(_) => self.remove(path),
        }
    }

    fn remove(&mut self, path: &Path) {
        if let Some(id) = self.tree.find(&self.root, path) {
            self.tree.remove(id);
        }
    }

    fn watch(&mut self, folder: &Path) {
        if let Some(watcher) = &self.watcher
            && let Err(error) = watcher.watch(folder)
        {
            tracing::warn!("cannot watch {}: {error}", folder.display());
            self.watcher = None;
        }
    }

    fn read_folder(&mut self, id: u32, folder: &Path, folders: &mut Vec<(u32, PathBuf)>) {
        if self.is_excluded(folder) {
            return;
        }
        let Ok(children) = fs::read_dir(folder) else {
            return;
        };
        if folder != self.root {
            self.watch(folder);
        }
        for child in children.flatten() {
            let Ok(kind) = child.file_type() else {
                continue;
            };
            let name = child.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if skipped_name(name) {
                continue;
            }
            if kind.is_dir() {
                let path = child.path();
                if self.is_excluded(&path) || path.join(CACHE_TAG).exists() {
                    continue;
                }
                if let Some(added) = self.tree.add(id, name, true) {
                    folders.push((added, path));
                }
            } else if kind.is_file() {
                self.tree.add(id, name, false);
            }
        }
    }

    fn search(&self, asked: &Asked) -> Vec<FileHit> {
        let Some(query) = Query::new(&asked.text) else {
            return Vec::new();
        };
        self.tree.search(
            &self.root,
            &query,
            asked.within.as_deref(),
            asked.limit,
            &HashMap::new(),
        )
    }
}
