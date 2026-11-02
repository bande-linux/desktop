use std::io;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;

use bande_content::{Content, content};
use bande_inbox::Inbox;

pub struct Loader {
    requests: Sender<(u64, PathBuf, String)>,
    loaded: Inbox<(u64, Content)>,
    generation: Arc<AtomicU64>,
}

impl Loader {
    pub fn new() -> io::Result<Self> {
        let loaded = Inbox::new()?;
        let outbox = loaded.outbox();
        let generation = Arc::new(AtomicU64::new(0));
        let latest = Arc::clone(&generation);
        let (requests, receiver) = mpsc::channel::<(u64, PathBuf, String)>();
        thread::Builder::new()
            .name("quick look".to_owned())
            .spawn(move || {
                for (asked, path, mime) in receiver {
                    let current = || latest.load(Ordering::Relaxed) == asked;
                    if !current() {
                        continue;
                    }
                    let content = content(&path, &mime, &current);
                    if current() && !outbox.send((asked, content)) {
                        return;
                    }
                }
            })?;
        Ok(Self {
            requests,
            loaded,
            generation,
        })
    }

    pub fn load(&self, path: PathBuf, mime: String) {
        let asked = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.requests.send((asked, path, mime));
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.loaded.fd()
    }

    pub fn take(&mut self) -> Vec<Content> {
        let latest = self.generation.load(Ordering::Relaxed);
        self.loaded
            .drain()
            .into_iter()
            .filter(|(asked, _)| *asked == latest)
            .map(|(_, content)| content)
            .collect()
    }
}
