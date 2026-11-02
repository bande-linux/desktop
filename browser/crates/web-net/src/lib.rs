mod cache;
mod cookies;
mod date;
mod executor;
mod fetch;
mod hsts;
mod mime;
mod reactor;
mod site;
mod storage;
mod stream;

use std::fmt;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use encoding_rs::UTF_8;
use rustix::event::{EventfdFlags, eventfd};
pub use url::Url;

pub use fetch::MAX_BODY;
pub use mime::ContentType;
pub use site::Site;
pub use storage::Storage;

use executor::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FetchId(u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub url: Url,
    pub top_level: Option<Url>,
    pub initiator: Option<Url>,
}

impl Request {
    pub fn navigation(url: Url, initiator: Option<Url>) -> Self {
        Self {
            url,
            top_level: None,
            initiator,
        }
    }

    pub fn subresource(url: Url, document: &Url) -> Self {
        Self {
            url,
            top_level: Some(document.clone()),
            initiator: Some(document.clone()),
        }
    }
}

#[derive(Debug)]
pub struct Response {
    pub url: Url,
    pub status: u16,
    pub content_type: Option<ContentType>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn text(&self) -> String {
        let encoding = self
            .content_type
            .as_ref()
            .and_then(|content_type| content_type.charset)
            .unwrap_or(UTF_8);
        encoding.decode(&self.body).0.into_owned()
    }
}

#[derive(Debug)]
pub enum FetchError {
    UnsupportedScheme,
    InvalidUrl,
    Connect(io::Error),
    Http(hyper::Error),
    TooManyRedirects,
    InvalidRedirect,
    TooLarge,
    BadEncoding,
    TimedOut,
}

impl fmt::Display for FetchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedScheme => formatter.write_str("the address scheme is not supported"),
            Self::InvalidUrl => formatter.write_str("the address is not valid"),
            Self::Connect(error) => write!(formatter, "cannot connect: {error}"),
            Self::Http(error) => match std::error::Error::source(error) {
                Some(source) => write!(formatter, "{error}: {source}"),
                None => write!(formatter, "{error}"),
            },
            Self::TooManyRedirects => formatter.write_str("too many redirects"),
            Self::InvalidRedirect => {
                formatter.write_str("the server redirected to an invalid address")
            }
            Self::TooLarge => formatter.write_str("the response is too large"),
            Self::BadEncoding => formatter.write_str("the response's content encoding is invalid"),
            Self::TimedOut => formatter.write_str("the server took too long to respond"),
        }
    }
}

impl std::error::Error for FetchError {}

pub type Completion = (FetchId, Result<Response, FetchError>);

pub struct Fetcher {
    commands: Sender<Command>,
    signal: Arc<OwnedFd>,
    ready: Arc<OwnedFd>,
    completions: Receiver<Completion>,
    next_id: u64,
    thread: Option<JoinHandle<()>>,
}

impl Fetcher {
    pub fn new() -> io::Result<Self> {
        Self::with_storage(Storage::Memory)
    }

    pub fn with_storage(storage: Storage) -> io::Result<Self> {
        let flags = EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK;
        let signal = Arc::new(eventfd(0, flags)?);
        let ready = Arc::new(eventfd(0, flags)?);
        let tls = executor::tls_config()?;
        let (commands, received) = mpsc::channel();
        let (completed, completions) = mpsc::channel();
        let thread = executor::spawn(
            received,
            Arc::clone(&signal),
            completed,
            Arc::clone(&ready),
            tls,
            storage,
        )?;
        Ok(Self {
            commands,
            signal,
            ready,
            completions,
            next_id: 0,
            thread: Some(thread),
        })
    }

    pub fn fetch(&mut self, url: Url) -> FetchId {
        self.request(Request::navigation(url, None))
    }

    pub fn request(&mut self, request: Request) -> FetchId {
        let id = FetchId(self.next_id);
        self.next_id += 1;
        self.send(Command::Fetch(id, Box::new(request)));
        id
    }

    pub fn cancel(&mut self, id: FetchId) {
        self.send(Command::Cancel(id));
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.ready.as_fd()
    }

    pub fn completed(&mut self) -> Vec<Completion> {
        executor::clear(&self.ready);
        self.completions.try_iter().collect()
    }

    fn send(&self, command: Command) {
        if self.commands.send(command).is_ok() {
            executor::signal(&self.signal);
        }
    }
}

impl Drop for Fetcher {
    fn drop(&mut self) {
        self.send(Command::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
