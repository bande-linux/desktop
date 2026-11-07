use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::path::{Path, PathBuf};

use crate::matcher::{Query, fold};
use crate::{FileHit, MAX_ENTRIES};

const NONE: u32 = u32::MAX;
pub const ROOT: u32 = 0;
const COMPACT_AFTER: usize = 1 << 20;

#[derive(Clone, Copy, Default)]
struct Text {
    start: u32,
    length: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stamp {
    pub inode: u64,
    pub modified: i64,
    pub size: u64,
    pub imported: bool,
}

impl Stamp {
    pub fn same_file(&self, other: &Self) -> bool {
        self.inode == other.inode && self.modified == other.modified && self.size == other.size
    }
}

struct Node {
    parent: u32,
    first_child: u32,
    next_sibling: u32,
    name: Text,
    folded: Text,
    folder: bool,
    removed: bool,
    stamp: Stamp,
}

impl Node {
    fn free() -> Self {
        Self {
            parent: NONE,
            first_child: NONE,
            next_sibling: NONE,
            name: Text::default(),
            folded: Text::default(),
            folder: false,
            removed: true,
            stamp: Stamp::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    pub id: u32,
    pub parent: u32,
    pub name: String,
    pub folder: bool,
    pub stamp: Stamp,
}

#[derive(Default)]
pub struct Tree {
    nodes: Vec<Node>,
    free: Vec<u32>,
    names: String,
    folded: String,
    live: usize,
    stale_bytes: usize,
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
    pub fn rooted() -> Self {
        let mut tree = Self::default();
        tree.nodes.push(Node {
            folder: true,
            removed: false,
            ..Node::free()
        });
        tree
    }

    pub fn restored(entries: Vec<Restored>) -> (Self, Vec<u32>) {
        let mut tree = Self::rooted();
        let top = entries.iter().map(|entry| entry.id).max().unwrap_or(ROOT);
        tree.nodes.resize_with(top as usize + 1, Node::free);
        for entry in &entries {
            let name = tree.named(&entry.name);
            if let (Some(node), Some((name, folded))) =
                (tree.nodes.get_mut(entry.id as usize), name)
            {
                *node = Node {
                    parent: entry.parent,
                    name,
                    folded,
                    folder: entry.folder,
                    removed: false,
                    stamp: entry.stamp,
                    ..Node::free()
                };
            }
        }
        let mut linked = vec![false; tree.nodes.len()];
        for entry in &entries {
            if let Some(seen) = linked.get_mut(entry.id as usize)
                && !std::mem::replace(seen, true)
            {
                tree.link(entry.id, entry.parent);
            }
        }
        let reachable = tree.subtree(ROOT);
        let mut kept = vec![false; tree.nodes.len()];
        for id in reachable {
            if let Some(slot) = kept.get_mut(id as usize) {
                *slot = true;
            }
        }
        let mut orphans = Vec::new();
        for entry in &entries {
            if !kept.get(entry.id as usize).copied().unwrap_or(false) {
                orphans.push(entry.id);
                if let Some(node) = tree.nodes.get_mut(entry.id as usize) {
                    *node = Node::free();
                }
            }
        }
        tree.live = tree.nodes.iter().filter(|node| !node.removed).count() - 1;
        tree.free = (1..=top)
            .rev()
            .filter(|&id| tree.node(id).is_none())
            .collect();
        (tree, orphans)
    }

    fn named(&mut self, name: &str) -> Option<(Text, Text)> {
        let lowered = fold(name);
        let folded = match lowered == name {
            true => Text::default(),
            false => stored(&mut self.folded, &lowered)?,
        };
        Some((stored(&mut self.names, name)?, folded))
    }

    fn link(&mut self, id: u32, parent: u32) {
        let Some(parent_node) = self.nodes.get_mut(parent as usize) else {
            return;
        };
        let next_sibling = std::mem::replace(&mut parent_node.first_child, id);
        if let Some(node) = self.nodes.get_mut(id as usize) {
            node.parent = parent;
            node.next_sibling = next_sibling;
        }
    }

    fn unlink(&mut self, id: u32) {
        let Some(parent) = self.node(id).map(|node| node.parent) else {
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
    }

    pub fn live(&self) -> usize {
        self.live
    }

    fn node(&self, id: u32) -> Option<&Node> {
        self.nodes.get(id as usize).filter(|node| !node.removed)
    }

    pub fn name(&self, id: u32) -> &str {
        self.node(id)
            .map_or("", |node| text(&self.names, node.name))
    }

    fn folded(&self, id: u32) -> &str {
        match self.node(id).map(|node| node.folded) {
            Some(folded) if folded.length > 0 => text(&self.folded, folded),
            _ => self.name(id),
        }
    }

    pub fn is_folder(&self, id: u32) -> bool {
        self.node(id).is_some_and(|node| node.folder)
    }

    pub fn stamp(&self, id: u32) -> Option<Stamp> {
        self.node(id).map(|node| node.stamp)
    }

    pub fn parent(&self, id: u32) -> Option<u32> {
        self.node(id)
            .map(|node| node.parent)
            .filter(|&parent| parent != NONE)
    }

    pub fn set_stamp(&mut self, id: u32, stamp: Stamp) {
        if let Some(node) = self.nodes.get_mut(id as usize).filter(|node| !node.removed) {
            node.stamp = stamp;
        }
    }

    pub fn set_folder(&mut self, id: u32, folder: bool) {
        if let Some(node) = self.nodes.get_mut(id as usize).filter(|node| !node.removed) {
            node.folder = folder;
        }
    }

    pub fn children(&self, id: u32) -> impl Iterator<Item = u32> + '_ {
        let first = self.node(id).map_or(NONE, |node| node.first_child);
        std::iter::successors(Some(first).filter(|&child| child != NONE), |&child| {
            self.node(child)
                .map(|node| node.next_sibling)
                .filter(|&next| next != NONE)
        })
    }

    pub fn child(&self, id: u32, name: &str) -> Option<u32> {
        self.children(id).find(|&child| self.name(child) == name)
    }

    pub fn descendants(&self, id: u32) -> impl Iterator<Item = u32> + '_ {
        let mut stack: Vec<u32> = self.children(id).collect();
        std::iter::from_fn(move || {
            let next = stack.pop()?;
            stack.extend(self.children(next));
            Some(next)
        })
    }

    pub fn subtree(&self, id: u32) -> Vec<u32> {
        std::iter::once(id).chain(self.descendants(id)).collect()
    }

    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        (1..u32::try_from(self.nodes.len()).unwrap_or(NONE)).filter(|&id| self.node(id).is_some())
    }

    fn depth(&self, id: u32) -> usize {
        std::iter::successors(Some(id), |&at| self.parent(at)).count()
    }

    pub fn insert(&mut self, parent: u32, name: &str, folder: bool) -> Option<u32> {
        match self.child(parent, name) {
            Some(known) => {
                self.set_folder(known, folder);
                Some(known)
            }
            None => self.add(parent, name, folder),
        }
    }

    pub fn add(&mut self, parent: u32, name: &str, folder: bool) -> Option<u32> {
        if self.live >= MAX_ENTRIES || self.node(parent).is_none() {
            return None;
        }
        let (name, folded) = self.named(name)?;
        let id = match self.free.pop() {
            Some(id) => id,
            None => {
                let id = u32::try_from(self.nodes.len())
                    .ok()
                    .filter(|&id| id != NONE)?;
                self.nodes.push(Node::free());
                id
            }
        };
        if let Some(node) = self.nodes.get_mut(id as usize) {
            *node = Node {
                name,
                folded,
                folder,
                removed: false,
                ..Node::free()
            };
        }
        self.link(id, parent);
        self.live += 1;
        Some(id)
    }

    pub fn relocate(&mut self, id: u32, parent: u32, name: &str) -> Option<()> {
        self.node(id)?;
        self.node(parent)?;
        if std::iter::successors(Some(parent), |&at| self.parent(at)).any(|at| at == id) {
            return None;
        }
        let (name, folded) = self.named(name)?;
        self.unlink(id);
        self.link(id, parent);
        let node = self.nodes.get_mut(id as usize)?;
        self.stale_bytes += (node.name.length + node.folded.length) as usize;
        node.name = name;
        node.folded = folded;
        self.compact_text();
        Some(())
    }

    pub fn remove(&mut self, id: u32) -> Vec<u32> {
        if id == ROOT || self.node(id).is_none() {
            return Vec::new();
        }
        self.unlink(id);
        let gone = self.subtree(id);
        for &at in &gone {
            if let Some(node) = self.nodes.get_mut(at as usize) {
                self.stale_bytes += (node.name.length + node.folded.length) as usize;
                *node = Node::free();
                self.free.push(at);
                self.live -= 1;
            }
        }
        self.compact_text();
        gone
    }

    fn compact_text(&mut self) {
        if self.stale_bytes < COMPACT_AFTER || self.stale_bytes < self.names.len() / 2 {
            return;
        }
        let (mut names, mut folded) = (String::new(), String::new());
        for id in 0..self.nodes.len() {
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            if node.removed {
                continue;
            }
            let (name, lowered) = (node.name, node.folded);
            let name = stored(&mut names, text(&self.names, name)).unwrap_or_default();
            let lowered = stored(&mut folded, text(&self.folded, lowered)).unwrap_or_default();
            if let Some(node) = self.nodes.get_mut(id) {
                node.name = name;
                node.folded = lowered;
            }
        }
        self.names = names;
        self.folded = folded;
        self.stale_bytes = 0;
    }

    pub fn path(&self, root: &Path, id: u32) -> PathBuf {
        let mut names: Vec<&str> = std::iter::successors(Some(id), |&at| self.parent(at))
            .map(|at| self.name(at))
            .collect();
        names.reverse();
        let mut path = root.to_path_buf();
        path.extend(names.into_iter().filter(|name| !name.is_empty()));
        path
    }

    pub fn find(&self, root: &Path, path: &Path) -> Option<u32> {
        path.strip_prefix(root)
            .ok()?
            .components()
            .try_fold(ROOT, |at, component| {
                self.child(at, component.as_os_str().to_str()?)
            })
    }

    pub fn search(
        &self,
        root: &Path,
        query: &Query,
        within: Option<&Path>,
        limit: usize,
        boosts: &HashMap<u32, u32>,
    ) -> Vec<FileHit> {
        let candidates: Box<dyn Iterator<Item = u32>> = match within {
            Some(within) => match self.find(root, within) {
                Some(id) => Box::new(self.descendants(id)),
                None => Box::new(std::iter::empty()),
            },
            None => Box::new(self.ids()),
        };
        let mut best = BinaryHeap::with_capacity(limit + 1);
        for id in candidates {
            let Some(score) = query.score(self.folded(id)) else {
                continue;
            };
            let score = score + boosts.get(&id).copied().unwrap_or(0);
            let depth = self.depth(id);
            if best.len() >= limit
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
                Reverse(self.path(root, id)),
                id,
            )));
            if best.len() > limit {
                best.pop();
            }
        }
        let mut hits: Vec<(usize, FileHit)> = best
            .into_iter()
            .map(|Reverse((score, Reverse(depth), Reverse(path), id))| {
                let hit = FileHit {
                    path,
                    folder: self.is_folder(id),
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
