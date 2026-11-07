use bande_volumes::on_connected_volume;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use bande_content::{Preview, image_size, preview, tree_size};
use bande_fs::Entry;
use bande_toolkit::Image;

#[derive(Debug)]
enum Request {
    Preview(Box<Entry>),
    FolderSize(PathBuf),
}

#[derive(Debug)]
pub enum Loaded {
    Preview {
        path: PathBuf,
        preview: Preview<Image>,
        pixels: Option<(u32, u32)>,
    },
    FolderSize {
        folder: PathBuf,
        items: u64,
        bytes: u64,
    },
}

#[derive(Debug)]
pub struct Loader {
    requests: Sender<(u64, Request)>,
    results: Receiver<Loaded>,
    generation: Arc<AtomicU64>,
    wake: UnixStream,
}

fn load_one(
    asked: u64,
    request: Request,
    latest: &AtomicU64,
    sender: &Sender<Loaded>,
    mut signal: &UnixStream,
) -> bool {
    let current = || latest.load(Ordering::Relaxed) == asked;
    let loaded = match request {
        Request::Preview(entry) => {
            let preview = preview(&entry, Image::from_rgba);
            let pixels = match preview {
                Preview::Image(_) => image_size(&entry.path),
                _ => None,
            };
            Some(Loaded::Preview {
                path: entry.path.clone(),
                preview,
                pixels,
            })
        }
        Request::FolderSize(folder) => {
            tree_size(&folder, &current).map(|(items, bytes)| Loaded::FolderSize {
                folder,
                items,
                bytes,
            })
        }
    };
    if let Some(loaded) = loaded {
        if sender.send(loaded).is_err() {
            return false;
        }
        let _ = signal.write(&[1]);
    }
    true
}

impl Loader {
    pub fn new() -> io::Result<Self> {
        let (wake, signal) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        signal.set_nonblocking(true)?;
        let (requests, receiver) = mpsc::channel::<(u64, Request)>();
        let (sender, results) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let latest = generation.clone();
        thread::Builder::new()
            .name("preview loader".to_owned())
            .spawn(move || {
                for (asked, request) in receiver {
                    if latest.load(Ordering::Relaxed) != asked {
                        continue;
                    }
                    let on_server = match &request {
                        Request::Preview(entry) => on_connected_volume(&entry.path),
                        Request::FolderSize(folder) => on_connected_volume(folder),
                    };
                    if !on_server {
                        if !load_one(asked, request, &latest, &sender, &signal) {
                            return;
                        }
                        continue;
                    }
                    let (latest, sender) = (Arc::clone(&latest), sender.clone());
                    let Ok(signal) = signal.try_clone() else {
                        continue;
                    };
                    let spawned = thread::Builder::new()
                        .name("preview on a server".to_owned())
                        .spawn(move || {
                            load_one(asked, request, &latest, &sender, &signal);
                        });
                    if let Err(error) = spawned {
                        tracing::warn!("cannot preview a file on a server: {error}");
                    }
                }
            })?;
        Ok(Self {
            requests,
            results,
            generation,
            wake,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    pub fn forget_pending(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn preview(&self, entry: Entry) {
        let generation = self.generation.load(Ordering::Relaxed);
        let _ = self
            .requests
            .send((generation, Request::Preview(Box::new(entry))));
    }

    pub fn folder_size(&self, folder: PathBuf) {
        let generation = self.generation.load(Ordering::Relaxed);
        let _ = self
            .requests
            .send((generation, Request::FolderSize(folder)));
    }

    pub fn take(&self) -> Vec<Loaded> {
        let mut drained = [0u8; 256];
        while matches!((&self.wake).read(&mut drained), Ok(read) if read > 0) {}
        self.results.try_iter().collect()
    }
}
