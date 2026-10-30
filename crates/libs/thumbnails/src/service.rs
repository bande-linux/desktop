use std::collections::{HashSet, VecDeque};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use crate::cache::{Cache, Picture, ThumbnailSize};
use crate::generate::{Thumbnailers, thumbnail};

const WORKERS: usize = 2;
pub const MAX_QUEUED: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Request {
    pub path: PathBuf,
    pub mime: String,
    pub size: ThumbnailSize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnailed {
    pub path: PathBuf,
    pub size: ThumbnailSize,
    pub picture: Option<Picture>,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Request>,
    busy: HashSet<(PathBuf, ThumbnailSize)>,
    closed: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub struct Thumbnails {
    shared: Arc<Shared>,
    results: Receiver<Thumbnailed>,
    wake: UnixStream,
}

struct Worker {
    shared: Arc<Shared>,
    cache: Arc<Cache>,
    thumbnailers: Arc<Thumbnailers>,
    results: Sender<Thumbnailed>,
    signal: Arc<UnixStream>,
}

impl Worker {
    fn next(&self) -> Option<Request> {
        let mut state = self.shared.lock();
        loop {
            if state.closed {
                return None;
            }
            if let Some(request) = state.queue.pop_front() {
                state.busy.insert((request.path.clone(), request.size));
                return Some(request);
            }
            state = self
                .shared
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn run(self) {
        while let Some(request) = self.next() {
            let picture = thumbnail(
                &self.cache,
                &self.thumbnailers,
                &request.path,
                &request.mime,
                request.size,
            );
            self.shared
                .lock()
                .busy
                .remove(&(request.path.clone(), request.size));
            let done = Thumbnailed {
                path: request.path,
                size: request.size,
                picture,
            };
            if self.results.send(done).is_err() {
                return;
            }
            let _ = (&*self.signal).write(&[1]);
        }
    }
}

impl Thumbnails {
    pub fn spawn(cache: Cache, thumbnailers: Thumbnailers) -> io::Result<Self> {
        let (wake, signal) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        signal.set_nonblocking(true)?;
        let (sender, results) = mpsc::channel();
        let shared = Arc::new(Shared::default());
        let (cache, thumbnailers, signal) =
            (Arc::new(cache), Arc::new(thumbnailers), Arc::new(signal));
        for index in 0..WORKERS {
            let worker = Worker {
                shared: shared.clone(),
                cache: cache.clone(),
                thumbnailers: thumbnailers.clone(),
                results: sender.clone(),
                signal: signal.clone(),
            };
            thread::Builder::new()
                .name(format!("thumbnails {index}"))
                .spawn(move || worker.run())?;
        }
        Ok(Self {
            shared,
            results,
            wake,
        })
    }

    pub fn for_user(data_dirs: &[PathBuf]) -> Option<Self> {
        let cache = Cache::for_user()?;
        Self::spawn(cache, Thumbnailers::load(data_dirs))
            .inspect_err(|error| tracing::warn!("cannot start making thumbnails: {error}"))
            .ok()
    }

    pub fn want(&self, requests: Vec<Request>) {
        let mut state = self.shared.lock();
        let busy = &state.busy;
        let mut seen = HashSet::new();
        let queue = requests
            .into_iter()
            .filter(|request| !busy.contains(&(request.path.clone(), request.size)))
            .filter(|request| seen.insert((request.path.clone(), request.size)))
            .take(MAX_QUEUED)
            .collect();
        state.queue = queue;
        drop(state);
        self.shared.ready.notify_all();
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    pub fn take(&self) -> Vec<Thumbnailed> {
        let mut drained = [0u8; 256];
        while matches!((&self.wake).read(&mut drained), Ok(read) if read > 0) {}
        self.results.try_iter().collect()
    }
}

impl Drop for Thumbnails {
    fn drop(&mut self) {
        self.shared.lock().closed = true;
        self.shared.ready.notify_all();
    }
}
