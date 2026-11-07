use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Weak};
use std::thread;

use bande_fs::{Entry, Listing, Sort, sort};

pub(crate) const FIRST_CHUNK: usize = 2_000;

pub(crate) type Ticket = Arc<u64>;

fn open_and_list(
    folder: &Path,
    show_hidden: bool,
    order: Sort,
    ticket: Weak<u64>,
    id: u64,
    sender: &Sender<Listed>,
    signal: &UnixStream,
) {
    match Listing::open(folder, show_hidden) {
        Ok(listing) => {
            list_rest(
                Request {
                    ticket,
                    id,
                    listing,
                    order,
                },
                sender,
                signal,
            );
        }
        Err(error) => {
            let _ = sender.send(Listed {
                id,
                entries: Vec::new(),
                done: true,
                failed: Some(error.kind()),
            });
            let _ = (&*signal).write(&[1]);
        }
    }
}

struct Request {
    ticket: Weak<u64>,
    id: u64,
    listing: Listing,
    order: Sort,
}

#[derive(Debug)]
pub(crate) struct Listed {
    pub id: u64,
    pub entries: Vec<Entry>,
    pub done: bool,
    pub failed: Option<io::ErrorKind>,
}

#[derive(Debug)]
pub(crate) struct Lister {
    requests: Sender<Request>,
    sender: Sender<Listed>,
    signal: UnixStream,
    results: Receiver<Listed>,
    wake: UnixStream,
    next: u64,
}

fn list_rest(request: Request, sender: &Sender<Listed>, signal: &UnixStream) -> bool {
    let Request {
        ticket,
        id,
        mut listing,
        order,
    } = request;
    let mut count = FIRST_CHUNK;
    while ticket.strong_count() > 0 {
        let mut entries = listing.next_chunk(count);
        sort(&mut entries, order);
        let done = listing.is_done();
        if sender
            .send(Listed {
                id,
                entries,
                done,
                failed: None,
            })
            .is_err()
        {
            return false;
        }
        let _ = (&*signal).write(&[1]);
        if done {
            break;
        }
        count *= 2;
    }
    true
}

impl Lister {
    pub(crate) fn new() -> io::Result<Self> {
        let (wake, signal) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        signal.set_nonblocking(true)?;
        let (requests, receiver) = mpsc::channel::<Request>();
        let (sender, results) = mpsc::channel();
        let (worker_sender, worker_signal) = (sender.clone(), signal.try_clone()?);
        thread::Builder::new()
            .name("folder lister".to_owned())
            .spawn(move || {
                for request in receiver {
                    if !list_rest(request, &worker_sender, &worker_signal) {
                        return;
                    }
                }
            })?;
        Ok(Self {
            requests,
            sender,
            signal,
            results,
            wake,
            next: 0,
        })
    }

    pub(crate) fn fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    pub(crate) fn finish(&mut self, listing: Listing, order: Sort) -> Ticket {
        self.next += 1;
        let ticket = Arc::new(self.next);
        let _ = self.requests.send(Request {
            ticket: Arc::downgrade(&ticket),
            id: self.next,
            listing,
            order,
        });
        ticket
    }

    pub(crate) fn open_on_its_own_thread(
        &mut self,
        folder: PathBuf,
        show_hidden: bool,
        order: Sort,
    ) -> io::Result<Ticket> {
        self.next += 1;
        let id = self.next;
        let ticket = Arc::new(id);
        let weak = Arc::downgrade(&ticket);
        let sender = self.sender.clone();
        let signal = self.signal.try_clone()?;
        thread::Builder::new()
            .name("connected folder".to_owned())
            .spawn(move || {
                open_and_list(&folder, show_hidden, order, weak, id, &sender, &signal);
            })?;
        Ok(ticket)
    }

    pub(crate) fn take(&self) -> Vec<Listed> {
        let mut drained = [0u8; 256];
        while matches!((&self.wake).read(&mut drained), Ok(read) if read > 0) {}
        self.results.try_iter().collect()
    }
}
