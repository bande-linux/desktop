use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Weak};
use std::thread;

use bande_fs::{Entry, Listing, Sort, sort};

pub(crate) const FIRST_CHUNK: usize = 2_000;

pub(crate) type Ticket = Arc<u64>;

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
}

#[derive(Debug)]
pub(crate) struct Lister {
    requests: Sender<Request>,
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
        if sender.send(Listed { id, entries, done }).is_err() {
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
        thread::Builder::new()
            .name("folder lister".to_owned())
            .spawn(move || {
                for request in receiver {
                    if !list_rest(request, &sender, &signal) {
                        return;
                    }
                }
            })?;
        Ok(Self {
            requests,
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

    pub(crate) fn take(&self) -> Vec<Listed> {
        let mut drained = [0u8; 256];
        while matches!((&self.wake).read(&mut drained), Ok(read) if read > 0) {}
        self.results.try_iter().collect()
    }
}
