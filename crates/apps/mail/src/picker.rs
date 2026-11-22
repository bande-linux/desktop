use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use bande_mail::portal::choose_files;

pub const MAX_ATTACHMENT: u64 = 25 << 20;

pub enum Picked {
    File(PathBuf, Vec<u8>),
    Refused(PathBuf, Refusal),
}

#[derive(Clone, Copy)]
pub enum Refusal {
    TooLarge,
    Folder,
    Unreadable,
}

pub struct Picker {
    sender: Sender<(u64, Vec<Picked>)>,
    received: Receiver<(u64, Vec<Picked>)>,
    awake: UnixStream,
    wake: Arc<UnixStream>,
}

impl Picker {
    pub fn new() -> std::io::Result<Self> {
        let (awake, wake) = UnixStream::pair()?;
        awake.set_nonblocking(true)?;
        let (sender, received) = mpsc::channel();
        Ok(Self {
            sender,
            received,
            awake,
            wake: Arc::new(wake),
        })
    }

    pub fn pick(&self, token: u64, title: String) {
        self.gather(token, move || {
            choose_files(&title)
                .inspect_err(|error| tracing::warn!("cannot choose files: {error}"))
                .unwrap_or_default()
        });
    }

    pub fn read(&self, token: u64, paths: Vec<PathBuf>) {
        self.gather(token, move || paths);
    }

    fn gather(&self, token: u64, paths: impl FnOnce() -> Vec<PathBuf> + Send + 'static) {
        let (sender, wake) = (self.sender.clone(), Arc::clone(&self.wake));
        let spawned = std::thread::Builder::new()
            .name("attach".to_owned())
            .spawn(move || {
                let picked = paths()
                    .into_iter()
                    .map(|path| {
                        let metadata = match std::fs::metadata(&path) {
                            Ok(metadata) => metadata,
                            Err(error) => {
                                tracing::warn!("cannot read {}: {error}", path.display());
                                return Picked::Refused(path, Refusal::Unreadable);
                            }
                        };
                        if metadata.is_dir() {
                            return Picked::Refused(path, Refusal::Folder);
                        }
                        if !metadata.is_file() {
                            return Picked::Refused(path, Refusal::Unreadable);
                        }
                        if metadata.len() > MAX_ATTACHMENT {
                            return Picked::Refused(path, Refusal::TooLarge);
                        }
                        let mut data = Vec::new();
                        match std::fs::File::open(&path)
                            .and_then(|file| file.take(MAX_ATTACHMENT + 1).read_to_end(&mut data))
                        {
                            Ok(_) if data.len() as u64 > MAX_ATTACHMENT => {
                                Picked::Refused(path, Refusal::TooLarge)
                            }
                            Ok(_) => Picked::File(path, data),
                            Err(error) => {
                                tracing::warn!("cannot read {}: {error}", path.display());
                                Picked::Refused(path, Refusal::Unreadable)
                            }
                        }
                    })
                    .collect();
                if sender.send((token, picked)).is_ok() {
                    let _ = (&*wake).write(&[1]);
                }
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot read attachments: {error}");
        }
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.awake.as_fd()
    }

    pub fn take(&mut self) -> Vec<(u64, Vec<Picked>)> {
        let mut buffer = [0u8; 64];
        while matches!((&self.awake).read(&mut buffer), Ok(count) if count > 0) {}
        self.received.try_iter().collect()
    }
}
