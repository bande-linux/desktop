use std::fs;
use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use bande_content::{Content, Contents, Decoded, Facts, Level, Pixels, content};
use bande_thumbnails::{Cache, ThumbnailSize, Thumbnailers, thumbnail};
use bande_xdg::{data_dirs, data_home};

use crate::worker::{Sample, read_archive, read_font_sample};

const FONTS: &str = "fonts";

pub enum Loaded {
    Content(Content),
    FontSample(Option<Sample>),
    Archive(Option<Contents>),
    FontInstalled(bool),
}
use bande_inbox::{Inbox, Outbox};

pub struct Loader {
    requests: Sender<(u64, PathBuf, String)>,
    loaded: Inbox<(u64, Loaded)>,
    generation: Arc<AtomicU64>,
}

fn thumbnailed(path: &Path, mime: &str) -> Option<Content> {
    let cache = Cache::for_user()?;
    let thumbnailers = Thumbnailers::load(&data_dirs());
    let picture = thumbnail(&cache, &thumbnailers, path, mime, ThumbnailSize::XxLarge)?;
    Some(Content::Picture(Decoded {
        width: picture.width,
        height: picture.height,
        pixels: Pixels::Still(vec![Level {
            width: picture.width,
            height: picture.height,
            rgba: Arc::new(picture.rgba),
        }]),
        facts: Facts::default(),
    }))
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
        content @ (Content::Other | Content::Failed(_)) => {
            Loaded::Content(thumbnailed(path, mime).unwrap_or(content))
        }
        content => Loaded::Content(content),
    };
    if current() {
        outbox.send((asked, loaded));
    }
}

pub fn installed_font(path: &Path) -> Option<PathBuf> {
    Some(data_home()?.join(FONTS).join(path.file_name()?))
}

fn install_font(path: &Path) -> io::Result<()> {
    let target = installed_font(path).ok_or_else(|| io::Error::other("no fonts folder"))?;
    let folder = target
        .parent()
        .ok_or_else(|| io::Error::other("no fonts folder"))?;
    fs::create_dir_all(folder)?;
    let mut partial = target.clone().into_os_string();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    fs::copy(path, &partial)
        .and_then(|_| fs::rename(&partial, &target))
        .inspect_err(|_| {
            let _ = fs::remove_file(&partial);
        })
}

fn serve(
    requests: &Receiver<(u64, PathBuf, String)>,
    latest: &AtomicU64,
    outbox: &Outbox<(u64, Loaded)>,
) {
    for (asked, path, mime) in requests {
        if latest.load(Ordering::Relaxed) == asked {
            load_one(asked, &path, &mime, latest, outbox);
        }
    }
}

impl Loader {
    pub fn new() -> io::Result<Self> {
        let loaded = Inbox::new()?;
        let outbox = loaded.outbox();
        let generation = Arc::new(AtomicU64::new(0));
        let latest = Arc::clone(&generation);
        let (requests, receiver) = mpsc::channel::<(u64, PathBuf, String)>();
        let (on_server, server_requests) = mpsc::channel::<(u64, PathBuf, String)>();
        let (server_latest, server_outbox) = (Arc::clone(&latest), outbox.clone());
        thread::Builder::new()
            .name("glance on a server".to_owned())
            .spawn(move || serve(&server_requests, &server_latest, &server_outbox))?;
        thread::Builder::new()
            .name("glance".to_owned())
            .spawn(move || {
                for (asked, path, mime) in receiver {
                    if latest.load(Ordering::Relaxed) != asked {
                        continue;
                    }
                    if !bande_volumes::on_connected_volume(&path) {
                        load_one(asked, &path, &mime, &latest, &outbox);
                    } else if on_server.send((asked, path, mime)).is_err() {
                        tracing::warn!("cannot preview a file on a server");
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

    pub fn install_font(&self, path: PathBuf) {
        let asked = self.generation.load(Ordering::Relaxed);
        let outbox = self.loaded.outbox();
        let spawned = thread::Builder::new()
            .name("font installer".to_owned())
            .spawn(move || {
                let installed = install_font(&path)
                    .inspect_err(|error| {
                        tracing::warn!("cannot install the font {}: {error}", path.display());
                    })
                    .is_ok();
                outbox.send((asked, Loaded::FontInstalled(installed)));
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot install a font: {error}");
            self.loaded
                .outbox()
                .send((asked, Loaded::FontInstalled(false)));
        }
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
