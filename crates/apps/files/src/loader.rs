use std::fs;
use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;

use unbound_fs::Entry;
use unbound_inbox::Inbox;
use unbound_preview::{Preview, preview};
use unbound_toolkit::Image;

#[derive(Debug)]
enum Request {
    Preview(Entry),
    FolderSize(PathBuf),
}

#[derive(Debug)]
pub enum Loaded {
    Preview(PathBuf, Preview<Image>),
    FolderSize {
        folder: PathBuf,
        items: u64,
        bytes: u64,
    },
}

#[derive(Debug)]
pub struct Loader {
    requests: Sender<(u64, Request)>,
    results: Inbox<Loaded>,
    generation: Arc<AtomicU64>,
}

fn tree_size(path: &Path, current: &dyn Fn() -> bool) -> Option<(u64, u64)> {
    let mut items = 0;
    let mut bytes = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(folder) = pending.pop() {
        if !current() {
            return None;
        }
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            items += 1;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                bytes += metadata.len();
            }
        }
    }
    Some((items, bytes))
}

impl Loader {
    pub fn new() -> io::Result<Self> {
        let (requests, receiver) = mpsc::channel::<(u64, Request)>();
        let results = Inbox::new()?;
        let sender = results.outbox();
        let generation = Arc::new(AtomicU64::new(0));
        let latest = generation.clone();
        thread::Builder::new()
            .name("preview loader".to_owned())
            .spawn(move || {
                for (asked, request) in receiver {
                    let current = || latest.load(Ordering::Relaxed) == asked;
                    if !current() {
                        continue;
                    }
                    let loaded = match request {
                        Request::Preview(entry) => Some(Loaded::Preview(
                            entry.path.clone(),
                            preview(&entry, Image::from_rgba),
                        )),
                        Request::FolderSize(folder) => {
                            tree_size(&folder, &current).map(|(items, bytes)| Loaded::FolderSize {
                                folder,
                                items,
                                bytes,
                            })
                        }
                    };
                    if let Some(loaded) = loaded
                        && !sender.send(loaded)
                    {
                        return;
                    }
                }
            })?;
        Ok(Self {
            requests,
            results,
            generation,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.results.fd()
    }

    pub fn forget_pending(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn preview(&self, entry: Entry) {
        let generation = self.generation.load(Ordering::Relaxed);
        let _ = self.requests.send((generation, Request::Preview(entry)));
    }

    pub fn folder_size(&self, folder: PathBuf) {
        let generation = self.generation.load(Ordering::Relaxed);
        let _ = self
            .requests
            .send((generation, Request::FolderSize(folder)));
    }

    pub fn take(&mut self) -> Vec<Loaded> {
        self.results.drain()
    }
}
