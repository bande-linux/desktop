mod broker;

use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use rustix::event::{EventfdFlags, eventfd};
pub use web_ipc::{Destination, Program, Size as Viewport, Url, Verdict};
use web_paint::{DisplayList, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Session {
    #[default]
    Default,
    Private(u32),
}

pub enum Command {
    Load {
        view: ViewId,
        load: LoadId,
        url: Url,
        viewport: Viewport,
        session: Session,
    },
    Resize {
        view: ViewId,
        viewport: Viewport,
    },
    Hover {
        view: ViewId,
        load: LoadId,
        x: f32,
        y: f32,
    },
    Fragment {
        view: ViewId,
        load: LoadId,
        name: String,
    },
    Stop {
        view: ViewId,
    },
    Close {
        view: ViewId,
    },
    Intercept(bool),
    Resume {
        request: u64,
        verdict: Verdict,
    },
    Quit,
}

pub struct Links {
    pub areas: Vec<(Rect, usize)>,
    pub targets: Vec<Url>,
}

impl Links {
    pub fn at(&self, x: f32, y: f32) -> Option<&Url> {
        self.areas
            .iter()
            .rev()
            .find(|(rect, _)| {
                x >= rect.x && y >= rect.y && x < rect.x + rect.width && y < rect.y + rect.height
            })
            .and_then(|(_, target)| self.targets.get(*target))
    }
}

pub struct Frame {
    pub url: Url,
    pub title: Option<String>,
    pub list: Arc<DisplayList>,
    pub links: Arc<Links>,
    pub fragment: Option<f32>,
}

pub struct PausedRequest {
    pub request: u64,
    pub url: Url,
    pub destination: Destination,
}

pub enum Outcome {
    Frame(Frame),
    Relaid(Arc<DisplayList>, Arc<Links>),
    Scroll(f32),
    Failed { url: Url, message: String },
    Unsupported { url: Url, kind: String },
    Crashed { url: Url },
    Paused(PausedRequest),
}

pub struct Update {
    pub view: ViewId,
    pub load: LoadId,
    pub outcome: Outcome,
}

pub struct Helpers {
    pub renderer: Program,
    pub network: Program,
}

pub type Placement = Box<dyn FnMut(u32) + Send>;

#[derive(Clone)]
pub struct Link {
    commands: Sender<Command>,
    signal: Arc<OwnedFd>,
}

impl Link {
    pub fn send(&self, command: Command) {
        if self.commands.send(command).is_ok() {
            signal(&self.signal);
        }
    }
}

pub struct Host {
    link: Link,
    ready: Arc<OwnedFd>,
    updates: Receiver<Update>,
    thread: Option<JoinHandle<()>>,
}

impl Host {
    pub fn new(helpers: Helpers, placement: Placement) -> io::Result<Self> {
        let flags = EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK;
        let signal = Arc::new(eventfd(0, flags)?);
        let ready = Arc::new(eventfd(0, flags)?);
        let (commands, received) = mpsc::channel();
        let (sent, updates) = mpsc::channel();
        let mut broker = broker::Broker::new(
            received,
            Arc::clone(&signal),
            sent,
            Arc::clone(&ready),
            helpers,
            placement,
        );
        let thread = std::thread::Builder::new()
            .name("web-host".into())
            .spawn(move || {
                if let Err(error) = broker.run() {
                    eprintln!("web-host: the broker stopped: {error}");
                }
            })?;
        Ok(Self {
            link: Link { commands, signal },
            ready,
            updates,
            thread: Some(thread),
        })
    }

    pub fn link(&self) -> Link {
        self.link.clone()
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.ready.as_fd()
    }

    pub fn updates(&self) -> Vec<Update> {
        clear(&self.ready);
        self.updates.try_iter().collect()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.link.send(Command::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn signal(fd: &OwnedFd) {
    let _ = rustix::io::write(fd, &1u64.to_ne_bytes());
}

pub(crate) fn clear(fd: &OwnedFd) {
    let mut count = [0; 8];
    let _ = rustix::io::read(fd, &mut count);
}
