use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;

use bande_content::{Content, Contents, content};

use crate::worker::{Sample, read_archive, read_font_sample};

pub enum Loaded {
    Content(Content),
    FontSample(Option<Sample>),
    Archive(Option<Contents>),
}
use bande_inbox::{Inbox, Outbox};

pub struct Loader {
    requests: Sender<(u64, PathBuf, String)>,
    loaded: Inbox<(u64, Loaded)>,
    generation: Arc<AtomicU64>,
}

fn load_one(
    asked: u64,
    path: &Path,
    mime: &str,
    latest: &AtomicU64,
    outbox: &Outbox<(u64, Loaded)>,
) {
    let current = || latest.load(Ordering::Relaxed) == asked;
    let loaded = match content(path, mime, &current) {
        Content::Font => Loaded::FontSample(read_font_sample(path)),
        Content::Archive => Loaded::Archive(read_archive(path)),
        content => Loaded::Content(content),
    };
    if current() {
        outbox.send((asked, loaded));
    }
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
                    if latest.load(Ordering::Relaxed) != asked {
                        continue;
                    }
                    if !bande_volumes::on_connected_volume(&path) {
                        load_one(asked, &path, &mime, &latest, &outbox);
                        continue;
                    }
                    let (latest, outbox) = (Arc::clone(&latest), outbox.clone());
                    let spawned = thread::Builder::new()
                        .name("quick look on a server".to_owned())
                        .spawn(move || load_one(asked, &path, &mime, &latest, &outbox));
                    if let Err(error) = spawned {
                        tracing::warn!("cannot preview a file on a server: {error}");
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

    pub fn take(&mut self) -> Vec<Loaded> {
        let latest = self.generation.load(Ordering::Relaxed);
        self.loaded
            .drain()
            .into_iter()
            .filter(|(asked, _)| *asked == latest)
            .map(|(_, content)| content)
            .collect()
    }
}
