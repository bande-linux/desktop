use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
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
const COMPACT_AFTER: usize = 4096;

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

    pub fn exclude(&self, excluded: Vec<PathBuf>) {
        let _ = self.commands.send(Command::Exclude(excluded));
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

const NONE: u32 = u32::MAX;
const ROOT: u32 = 0;

#[derive(Clone, Copy, Default)]
struct Text {
    start: u32,
    length: u32,
}

struct Node {
    parent: u32,
    first_child: u32,
    next_sibling: u32,
    name: Text,
    folded: Text,
    folder: bool,
    removed: bool,
}

#[derive(Default)]
struct Tree {
    nodes: Vec<Node>,
    names: String,
    folded: String,
    removed: usize,
}

fn stored(arena: &mut String, text: &str) -> Option<Text> {
    let start = u32::try_from(arena.len()).ok()?;
    let length = u32::try_from(text.len()).ok()?;
    arena.push_str(text);
    Some(Text { start, length })
}

fn text(arena: &str, text: Text) -> &str {
    let start = text.start as usize;
    arena
        .get(start..start + text.length as usize)
        .unwrap_or_default()
}

impl Tree {
    fn rooted() -> Self {
        let mut tree = Self::default();
        tree.nodes.push(Node {
            parent: NONE,
            first_child: NONE,
            next_sibling: NONE,
            name: Text::default(),
            folded: Text::default(),
            folder: true,
            removed: false,
        });
        tree
    }

    fn live(&self) -> usize {
        self.nodes.len() - 1 - self.removed
    }

    fn node(&self, id: u32) -> Option<&Node> {
        self.nodes.get(id as usize)
    }

    fn name(&self, id: u32) -> &str {
        self.node(id)
            .map_or("", |node| text(&self.names, node.name))
    }

    fn folded(&self, id: u32) -> &str {
        match self.node(id).map(|node| node.folded) {
            Some(folded) if folded.length > 0 => text(&self.folded, folded),
            _ => self.name(id),
        }
    }

    fn children(&self, id: u32) -> impl Iterator<Item = u32> + '_ {
        let first = self.node(id).map_or(NONE, |node| node.first_child);
        std::iter::successors(Some(first).filter(|&child| child != NONE), |&child| {
            self.node(child)
                .map(|node| node.next_sibling)
                .filter(|&next| next != NONE)
        })
    }

    fn child(&self, id: u32, name: &str) -> Option<u32> {
        self.children(id).find(|&child| self.name(child) == name)
    }

    fn descendants(&self, id: u32) -> impl Iterator<Item = u32> + '_ {
        let mut stack: Vec<u32> = self.children(id).collect();
        std::iter::from_fn(move || {
            let next = stack.pop()?;
            stack.extend(self.children(next));
            Some(next)
        })
    }

    fn depth(&self, id: u32) -> usize {
        std::iter::successors(Some(id), |&at| {
            self.node(at)
                .map(|node| node.parent)
                .filter(|&parent| parent != NONE)
        })
        .count()
    }

    fn insert(&mut self, parent: u32, name: &str, folder: bool) -> Option<u32> {
        match self.child(parent, name) {
            Some(known) => {
                if let Some(node) = self.nodes.get_mut(known as usize) {
                    node.folder = folder;
                }
                Some(known)
            }
            None => self.add(parent, name, folder),
        }
    }

    fn add(&mut self, parent: u32, name: &str, folder: bool) -> Option<u32> {
        if self.live() >= MAX_ENTRIES {
            return None;
        }
        let id = u32::try_from(self.nodes.len())
            .ok()
            .filter(|&id| id != NONE)?;
        let lowered = fold(name);
        let folded = match lowered == name {
            true => Text::default(),
            false => stored(&mut self.folded, &lowered)?,
        };
        let name = stored(&mut self.names, name)?;
        let sibling = self.nodes.get_mut(parent as usize)?;
        let next_sibling = std::mem::replace(&mut sibling.first_child, id);
        self.nodes.push(Node {
            parent,
            first_child: NONE,
            next_sibling,
            name,
            folded,
            folder,
            removed: false,
        });
        Some(id)
    }

    fn remove(&mut self, id: u32) {
        let Some(parent) = self.node(id).filter(|_| id != ROOT).map(|node| node.parent) else {
            return;
        };
        let next = self.node(id).map_or(NONE, |node| node.next_sibling);
        let previous = self
            .children(parent)
            .find(|&child| self.node(child).is_some_and(|node| node.next_sibling == id));
        let link = match previous {
            Some(previous) => self
                .nodes
                .get_mut(previous as usize)
                .map(|node| &mut node.next_sibling),
            None => self
                .nodes
                .get_mut(parent as usize)
                .map(|node| &mut node.first_child),
        };
        if let Some(link) = link {
            *link = next;
        }
        let gone: Vec<u32> = std::iter::once(id).chain(self.descendants(id)).collect();
        for at in gone {
            if let Some(node) = self.nodes.get_mut(at as usize) {
                node.removed = true;
                self.removed += 1;
            }
        }
        if self.removed > COMPACT_AFTER && self.removed > self.live() {
            self.compact();
        }
    }

    fn compact(&mut self) {
        let mut compact = Self::rooted();
        let mut stack = vec![(ROOT, ROOT)];
        while let Some((old, new)) = stack.pop() {
            let children: Vec<u32> = self.children(old).collect();
            for child in children.into_iter().rev() {
                let folder = self.node(child).is_some_and(|node| node.folder);
                if let Some(copied) = compact.add(new, self.name(child), folder) {
                    stack.push((child, copied));
                }
            }
        }
        *self = compact;
    }

    fn path(&self, root: &Path, id: u32) -> PathBuf {
        let mut names: Vec<&str> = std::iter::successors(Some(id), |&at| {
            self.node(at)
                .map(|node| node.parent)
                .filter(|&parent| parent != NONE)
        })
        .map(|at| self.name(at))
        .collect();
        names.reverse();
        let mut path = root.to_path_buf();
        path.extend(names.into_iter().filter(|name| !name.is_empty()));
        path
    }

    fn find(&self, root: &Path, path: &Path) -> Option<u32> {
        path.strip_prefix(root)
            .ok()?
            .components()
            .try_fold(ROOT, |at, component| {
                self.child(at, component.as_os_str().to_str()?)
            })
    }
}

struct Worker {
    root: PathBuf,
    excluded: Vec<PathBuf>,
    tree: Tree,
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
        self.excluded.iter().any(|folder| path.starts_with(folder))
    }

    fn indexed(&self, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(&self.root) else {
            return false;
        };
        !self.is_excluded(path)
            && relative
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
            if skipped(name) {
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
        let tree = &self.tree;
        let boosts: HashMap<u32, u32> = self
            .boosts
            .iter()
            .filter_map(|(path, boost)| Some((tree.find(&self.root, path)?, *boost)))
            .collect();
        let candidates: Box<dyn Iterator<Item = u32>> = match &asked.within {
            Some(within) => match tree.find(&self.root, within) {
                Some(id) => Box::new(tree.descendants(id)),
                None => Box::new(std::iter::empty()),
            },
            None => Box::new(
                (1..u32::try_from(tree.nodes.len()).unwrap_or(NONE))
                    .filter(|&id| tree.node(id).is_some_and(|node| !node.removed)),
            ),
        };
        let mut best = BinaryHeap::with_capacity(asked.limit + 1);
        for id in candidates {
            let Some(score) = query.score(tree.folded(id)) else {
                continue;
            };
            let score = score + boosts.get(&id).copied().unwrap_or(0);
            let depth = tree.depth(id);
            if best.len() >= asked.limit
                && best
                    .peek()
                    .is_some_and(|Reverse((lowest, Reverse(deepest), _, _))| {
                        (score, Reverse(depth)) < (*lowest, Reverse(*deepest))
                    })
            {
                continue;
            }
            best.push(Reverse((
                score,
                Reverse(depth),
                Reverse(tree.path(&self.root, id)),
                id,
            )));
            if best.len() > asked.limit {
                best.pop();
            }
        }
        let mut hits: Vec<(usize, FileHit)> = best
            .into_iter()
            .map(|Reverse((score, Reverse(depth), Reverse(path), id))| {
                let hit = FileHit {
                    path,
                    folder: tree.node(id).is_some_and(|node| node.folder),
                    score,
                };
                (depth, hit)
            })
            .collect();
        hits.sort_by(|(a_depth, a), (b_depth, b)| {
            b.score
                .cmp(&a.score)
                .then_with(|| a_depth.cmp(b_depth))
                .then_with(|| a.path.cmp(&b.path))
        });
        hits.into_iter().map(|(_, hit)| hit).collect()
    }
}
