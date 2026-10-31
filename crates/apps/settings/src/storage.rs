use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};

use unbound_fs::Trash;

use crate::failure::unexpected;
use crate::follow::{Update, Updates, spawn};

const BLOCK: u64 = 512;
pub const LARGE_FILES: usize = 8;
const APP_FOLDERS: [&str; 3] = ["/usr", "/opt", "/var/lib/flatpak"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Apps,
    Documents,
    Trash,
    System,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Storage {
    pub capacity: u64,
    pub available: u64,
    pub categories: Option<Vec<(Category, u64)>>,
    pub largest: Vec<(PathBuf, u64)>,
    pub trash_items: usize,
}

impl Storage {
    pub fn used(&self) -> u64 {
        self.capacity.saturating_sub(self.available)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageCommand {
    Measure,
    EmptyTrash,
}

#[derive(Debug, Default)]
pub struct Tally {
    pub bytes: u64,
    pub largest: Vec<(PathBuf, u64)>,
}

impl Tally {
    fn add_file(&mut self, path: &Path, bytes: u64) {
        self.bytes += bytes;
        if self.largest.len() < LARGE_FILES
            || self
                .largest
                .last()
                .is_some_and(|(_, smallest)| bytes > *smallest)
        {
            let at = self.largest.partition_point(|(_, known)| *known >= bytes);
            self.largest.insert(at, (path.to_path_buf(), bytes));
            self.largest.truncate(LARGE_FILES);
        }
    }
}

pub fn tally(root: &Path, skip: &[&Path]) -> Tally {
    let mut tally = Tally::default();
    let Ok(top) = fs::symlink_metadata(root) else {
        return tally;
    };
    let device = top.dev();
    let mut pending = vec![root.to_path_buf()];
    while let Some(folder) = pending.pop() {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.dev() != device || skip.contains(&path.as_path()) {
                continue;
            }
            if metadata.is_dir() {
                tally.bytes += metadata.blocks() * BLOCK;
                pending.push(path);
            } else if metadata.is_file() {
                tally.add_file(&path, metadata.blocks() * BLOCK);
            }
        }
    }
    tally
}

pub(crate) fn disk(root: &Path) -> (u64, u64) {
    rustix::fs::statvfs(root).map_or((0, 0), |stats| {
        (
            stats.f_blocks * stats.f_frsize,
            stats.f_bavail * stats.f_frsize,
        )
    })
}

pub fn categorize(used: u64, apps: u64, documents: u64, trash: u64) -> Vec<(Category, u64)> {
    let system = used.saturating_sub(apps + documents + trash);
    [
        (Category::Apps, apps),
        (Category::Documents, documents),
        (Category::Trash, trash),
        (Category::System, system),
    ]
    .into_iter()
    .filter(|(_, bytes)| *bytes > 0)
    .collect()
}

fn measure(home: &Path, trash: &Trash, updates: &Updates) {
    let (capacity, available) = disk(home);
    let trash_items = trash.items().len();
    let mut storage = Storage {
        capacity,
        available,
        categories: None,
        largest: Vec::new(),
        trash_items,
    };
    if !updates.send(Update::Storage(storage.clone())) {
        return;
    }
    let trash_root = trash.home().root().to_path_buf();
    let trash_bytes = tally(&trash_root, &[]).bytes;
    let documents = tally(home, &[&trash_root]);
    let apps: u64 = APP_FOLDERS
        .iter()
        .map(|folder| tally(Path::new(folder), &[]).bytes)
        .sum();
    storage.categories = Some(categorize(
        storage.used(),
        apps,
        documents.bytes,
        trash_bytes,
    ));
    storage.largest = documents.largest;
    updates.send(Update::Storage(storage));
}

pub fn follow_storage(updates: Updates) -> Sender<StorageCommand> {
    let (sender, receiver) = mpsc::channel();
    spawn("storage", move || {
        let (Some(home), Some(trash)) = (std::env::home_dir(), Trash::for_user()) else {
            return;
        };
        while let Ok(first) = receiver.recv() {
            let mut commands = vec![first];
            commands.extend(receiver.try_iter());
            if commands.contains(&StorageCommand::EmptyTrash)
                && let Err(error) = trash.empty()
            {
                updates.send(unexpected(error));
            }
            measure(&home, &trash, &updates);
        }
    });
    sender
}
