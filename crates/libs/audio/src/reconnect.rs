use std::ffi::OsString;
use std::mem::MaybeUninit;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use pipewire as pw;
use rustix::event::{EventfdFlags, PollFd, PollFlags, Timespec, eventfd, poll};
use rustix::fs::inotify;
use rustix::io::write;

use crate::Error;
use crate::graph::{self, Command, Shared};

const DEFAULT_SOCKET: &str = "pipewire-0";
const RETRIES_AFTER_A_CHANGE: [Duration; 3] = [
    Duration::from_millis(100),
    Duration::from_millis(500),
    Duration::from_secs(2),
];
const STEADY_CONNECTION: Duration = Duration::from_secs(1);

pub(crate) struct Link {
    commands: Option<pw::channel::Sender<Command>>,
    stop: Arc<OwnedFd>,
}

pub(crate) fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Link {
    pub(crate) fn new() -> Result<Self, Error> {
        let stop = eventfd(0, EventfdFlags::CLOEXEC).map_err(os_error)?;
        Ok(Self {
            commands: None,
            stop: Arc::new(stop),
        })
    }

    pub(crate) fn commands(&self) -> Option<pw::channel::Sender<Command>> {
        self.commands.clone()
    }

    pub(crate) fn stop(&self) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(Command::Quit);
        }
        if let Err(error) = write(&self.stop, &1_u64.to_ne_bytes()) {
            tracing::warn!("cannot end the PipeWire connection: {error}");
        }
    }
}

fn os_error(error: rustix::io::Errno) -> Error {
    Error::PipeWire(error.to_string())
}

enum Wake {
    Stop,
    SocketChanged,
    Later,
}

struct SocketWatch {
    inotify: Option<OwnedFd>,
    socket: OsString,
}

impl SocketWatch {
    fn new() -> Self {
        let remote = std::env::var_os("PIPEWIRE_REMOTE").filter(|name| !name.is_empty());
        let directory = std::env::var_os("PIPEWIRE_RUNTIME_DIR")
            .or_else(|| std::env::var_os("XDG_RUNTIME_DIR"))
            .map(PathBuf::from);
        let inotify = directory.and_then(|directory| {
            let watch =
                inotify::init(inotify::CreateFlags::CLOEXEC | inotify::CreateFlags::NONBLOCK)
                    .and_then(|watch| {
                        inotify::add_watch(
                            &watch,
                            &directory,
                            inotify::WatchFlags::CREATE | inotify::WatchFlags::MOVED_TO,
                        )?;
                        Ok(watch)
                    });
            watch
                .inspect_err(|error| {
                    tracing::warn!("cannot watch for the PipeWire socket: {error}");
                })
                .ok()
        });
        Self {
            inotify,
            socket: remote.unwrap_or_else(|| DEFAULT_SOCKET.into()),
        }
    }

    fn socket_changed(&self) -> bool {
        let Some(watch) = &self.inotify else {
            return false;
        };
        let mut buffer = [MaybeUninit::uninit(); 1024];
        let mut events = inotify::Reader::new(watch, &mut buffer);
        let mut changed = false;
        while let Ok(event) = events.next() {
            changed |= event
                .file_name()
                .is_some_and(|name| name.to_bytes() == self.socket.as_encoded_bytes());
        }
        changed
    }

    fn wait(&self, stop: &OwnedFd, until: Option<Instant>) -> Wake {
        loop {
            let mut fds = vec![PollFd::new(stop, PollFlags::IN)];
            if let Some(watch) = &self.inotify {
                fds.push(PollFd::new(watch, PollFlags::IN));
            }
            let timeout = until.map(|until| {
                Timespec::try_from(until.saturating_duration_since(Instant::now())).unwrap_or(
                    Timespec {
                        tv_sec: 0,
                        tv_nsec: 0,
                    },
                )
            });
            let polled = poll(&mut fds, timeout.as_ref());
            let stopping = fds[0].revents().contains(PollFlags::IN);
            drop(fds);
            match polled {
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => {
                    tracing::warn!("cannot wait for PipeWire: {error}");
                    return Wake::Stop;
                }
                Ok(0) => return Wake::Later,
                Ok(_) if stopping => return Wake::Stop,
                Ok(_) if self.socket_changed() => return Wake::SocketChanged,
                Ok(_) => {}
            }
        }
    }
}

fn publish(link: &Mutex<Link>) -> pw::channel::Receiver<Command> {
    let (commands, receiver) = pw::channel::channel();
    locked(link).commands = Some(commands);
    receiver
}

pub(crate) fn keep_connected(shared: &Arc<Shared>, link: &Mutex<Link>) {
    let stop = Arc::clone(&locked(link).stop);
    let watch = SocketWatch::new();
    let mut retries = RETRIES_AFTER_A_CHANGE.iter();
    let mut logged = false;
    loop {
        let receiver = publish(link);
        let Some(sender) = locked(link).commands() else {
            return;
        };
        if shared.stopping.load(Ordering::SeqCst) {
            return;
        }
        let started = Instant::now();
        let outcome = graph::run(shared, receiver, sender);
        locked(link).commands = None;
        if shared.stopping.load(Ordering::SeqCst) {
            return;
        }
        let steady = started.elapsed() >= STEADY_CONNECTION;
        match outcome {
            Ok(()) if steady => {
                retries = RETRIES_AFTER_A_CHANGE.iter();
                logged = false;
                continue;
            }
            Ok(()) => {}
            Err(error) if !logged => {
                logged = true;
                tracing::warn!("no sound until PipeWire answers: {error}");
            }
            Err(_) => {}
        }
        let until = retries.next().map(|delay| Instant::now() + *delay);
        match watch.wait(&stop, until) {
            Wake::Stop => return,
            Wake::SocketChanged => retries = RETRIES_AFTER_A_CHANGE.iter(),
            Wake::Later => {}
        }
    }
}
