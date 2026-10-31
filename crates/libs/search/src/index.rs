use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap};
use std::fs;
use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread;

use bande_inbox::{Inbox, Outbox};
use bande_watch::{Changes, FolderWatch};

use crate::matcher::{Query, fold};

pub const MAX_HITS: usize = 40;
const MAX_ENTRIES: usize = 400_000;
const FOLDERS_PER_BATCH: usize = 64;
const SKIPPED_FOLDERS: &[&str] = &["node_modules", "__pycache__", "snap"];
const CACHE_TAG: &str = "CACHEDIR.TAG";

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
    Boosts(HashMap<PathBuf, u32>),
    Changed(Vec<PathBuf>),
}

pub struct FileIndex {
    commands: Sender<Command>,
    results: Inbox<FileResults>,
}

impl FileIndex {
    pub fn spawn(root: PathBuf) -> io::Result<Self> {
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
                    entries: BTreeMap::new(),
                    watcher,
                    boosts: HashMap::new(),
                    commands: received,
                    results: sender,
                    pending: None,
                    answered: None,
                };
                worker.run();
            })?;
        Ok(Self { commands, results })
    }

    pub fn query(&self, generation: u64, text: &str) {
        let _ = self.commands.send(Command::Query(Asked {
            generation,
            text: text.to_owned(),
            within: None,
            limit: MAX_HITS,
        }));
    }

    pub fn query_within(&self, generation: u64, text: &str, within: &Path, limit: usize) {
        let _ = self.commands.send(Command::Query(Asked {
            generation,
            text: text.to_owned(),
            within: Some(within.to_path_buf()),
            limit,
        }));
    }

    pub fn set_boosts(&self, boosts: HashMap<PathBuf, u32>) {
        let _ = self.commands.send(Command::Boosts(boosts));
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.results.fd()
    }

    pub fn take(&mut self) -> Option<FileResults> {
        self.results.drain().pop()
    }
}

struct Entry {
    folded: Box<str>,
    folder: bool,
}

struct Worker {
    root: PathBuf,
    entries: BTreeMap<PathBuf, Entry>,
    watcher: Option<FolderWatch>,
    boosts: HashMap<PathBuf, u32>,
    commands: Receiver<Command>,
    results: Outbox<FileResults>,
    pending: Option<Asked>,
    answered: Option<Asked>,
}

fn skipped(name: &str) -> bool {
    name.starts_with('.') || SKIPPED_FOLDERS.contains(&name)
}

fn file_name(path: &Path) -> Option<&str> {
    path.file_name()?.to_str()
}

impl Worker {
    fn run(&mut self) {
        let mut folders = vec![self.root.clone()];
        self.watch(&self.root.clone());
        let mut read = 0usize;
        while let Some(folder) = folders.pop() {
            self.read_folder(&folder, &mut folders);
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
            if !self.results.send(FileResults { generation, hits }) {
                return false;
            }
        }
        true
    }

    fn apply(&mut self, command: Command) {
        match command {
            Command::Query(asked) => self.pending = Some(asked),
            Command::Boosts(boosts) => self.boosts = boosts,
            Command::Changed(paths) => {
                for path in paths {
                    self.changed(&path);
                }
            }
        }
    }

    fn indexed(&self, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(&self.root) else {
            return false;
        };
        relative
            .components()
            .all(|component| !component.as_os_str().to_str().is_none_or(skipped))
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
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_dir() => {
                if !self.entries.contains_key(path) && self.insert(path, true) {
                    let mut folders = vec![path.to_path_buf()];
                    while let Some(folder) = folders.pop() {
                        self.read_folder(&folder, &mut folders);
                    }
                }
            }
            Ok(metadata) if metadata.is_file() => {
                self.insert(path, false);
            }
            Ok(_) => {}
            Err(_) => self.remove(path),
        }
    }

    fn remove(&mut self, path: &Path) {
        let removed: Vec<PathBuf> = self
            .entries
            .range(path.to_path_buf()..)
            .map(|(key, _)| key)
            .take_while(|key| key.starts_with(path))
            .cloned()
            .collect();
        for key in removed {
            self.entries.remove(&key);
        }
    }

    fn insert(&mut self, path: &Path, folder: bool) -> bool {
        if self.entries.len() >= MAX_ENTRIES {
            return false;
        }
        let Some(name) = file_name(path) else {
            return false;
        };
        self.entries.insert(
            path.to_path_buf(),
            Entry {
                folded: fold(name).into_boxed_str(),
                folder,
            },
        );
        true
    }

    fn watch(&mut self, folder: &Path) {
        if let Some(watcher) = &self.watcher
            && let Err(error) = watcher.watch(folder)
        {
            tracing::warn!("cannot watch {}: {error}", folder.display());
            self.watcher = None;
        }
    }

    fn read_folder(&mut self, folder: &Path, folders: &mut Vec<PathBuf>) {
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
            if skipped(name) {
                continue;
            }
            let path = child.path();
            if kind.is_dir() {
                if path.join(CACHE_TAG).exists() {
                    continue;
                }
                if self.insert(&path, true) {
                    folders.push(path);
                }
            } else if kind.is_file() {
                self.insert(&path, false);
            }
        }
    }

    fn search(&self, asked: &Asked) -> Vec<FileHit> {
        let Some(query) = Query::new(&asked.text) else {
            return Vec::new();
        };
        let mut best = BinaryHeap::with_capacity(asked.limit + 1);
        let entries: Box<dyn Iterator<Item = (&PathBuf, &Entry)>> = match &asked.within {
            Some(within) => Box::new(
                self.entries
                    .range(within.clone()..)
                    .take_while(move |(path, _)| path.starts_with(within))
                    .filter(move |(path, _)| *path != within),
            ),
            None => Box::new(self.entries.iter()),
        };
        for (path, entry) in entries {
            let Some(score) = query.score(&entry.folded) else {
                continue;
            };
            let score = score + self.boosts.get(path).copied().unwrap_or(0);
            let depth = path.components().count();
            best.push(Reverse((score, Reverse(depth), Reverse(path))));
            if best.len() > asked.limit {
                best.pop();
            }
        }
        let mut hits: Vec<FileHit> = best
            .into_iter()
            .map(|Reverse((score, _, Reverse(path)))| FileHit {
                path: path.clone(),
                folder: self.entries.get(path).is_some_and(|entry| entry.folder),
                score,
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| {
                    a.path
                        .components()
                        .count()
                        .cmp(&b.path.components().count())
                })
                .then_with(|| a.path.cmp(&b.path))
        });
        hits
    }
}
