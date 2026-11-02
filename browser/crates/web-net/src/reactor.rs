use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io;
use std::net::TcpStream;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use rustix::buffer::spare_capacity;
use rustix::event::{Timespec, epoll};

const SIGNAL_KEY: u64 = 0;
const EVENT_BATCH: usize = 64;

#[derive(Default)]
struct Interest {
    read: Option<Waker>,
    write: Option<Waker>,
}

pub(crate) struct Reactor {
    epoll: OwnedFd,
    interests: RefCell<HashMap<u64, Interest>>,
    next_key: Cell<u64>,
}

impl Reactor {
    pub(crate) fn new(signal: BorrowedFd<'_>) -> io::Result<Rc<Self>> {
        let epoll = epoll::create(epoll::CreateFlags::CLOEXEC)?;
        epoll::add(
            &epoll,
            signal,
            epoll::EventData::new_u64(SIGNAL_KEY),
            epoll::EventFlags::IN,
        )?;
        Ok(Rc::new(Self {
            epoll,
            interests: RefCell::new(HashMap::new()),
            next_key: Cell::new(SIGNAL_KEY + 1),
        }))
    }

    pub(crate) fn wait(&self, timeout: Option<Duration>) -> io::Result<()> {
        let timeout = timeout.map(Timespec::try_from).transpose().ok().flatten();
        let mut events = Vec::with_capacity(EVENT_BATCH);
        match epoll::wait(&self.epoll, spare_capacity(&mut events), timeout.as_ref()) {
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => return Ok(()),
            Err(error) => return Err(error.into()),
        }
        let mut interests = self.interests.borrow_mut();
        let mut wakers = Vec::new();
        for event in &events {
            let Some(interest) = interests.get_mut(&event.data.u64()) else {
                continue;
            };
            let flags = event.flags;
            let failed = flags.intersects(epoll::EventFlags::ERR | epoll::EventFlags::HUP);
            if failed || flags.intersects(epoll::EventFlags::IN | epoll::EventFlags::RDHUP) {
                wakers.extend(interest.read.take());
            }
            if failed || flags.contains(epoll::EventFlags::OUT) {
                wakers.extend(interest.write.take());
            }
        }
        drop(interests);
        wakers.into_iter().for_each(Waker::wake);
        Ok(())
    }

    fn register(self: &Rc<Self>, fd: BorrowedFd<'_>) -> io::Result<u64> {
        let key = self.next_key.get();
        self.next_key.set(key + 1);
        epoll::add(
            &self.epoll,
            fd,
            epoll::EventData::new_u64(key),
            epoll::EventFlags::IN
                | epoll::EventFlags::OUT
                | epoll::EventFlags::RDHUP
                | epoll::EventFlags::ET,
        )?;
        self.interests.borrow_mut().insert(key, Interest::default());
        Ok(key)
    }

    fn wait_for(&self, key: u64, direction: Direction, waker: &Waker) {
        if let Some(interest) = self.interests.borrow_mut().get_mut(&key) {
            let slot = match direction {
                Direction::Read => &mut interest.read,
                Direction::Write => &mut interest.write,
            };
            *slot = Some(waker.clone());
        }
    }

    fn forget(&self, key: u64, fd: BorrowedFd<'_>) {
        self.interests.borrow_mut().remove(&key);
        let _ = epoll::delete(&self.epoll, fd);
    }
}

#[derive(Clone, Copy)]
enum Direction {
    Read,
    Write,
}

pub(crate) struct Socket {
    stream: TcpStream,
    key: u64,
    reactor: Rc<Reactor>,
}

impl Socket {
    pub(crate) fn new(stream: TcpStream, reactor: &Rc<Reactor>) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        let key = reactor.register(stream.as_fd())?;
        Ok(Self {
            stream,
            key,
            reactor: Rc::clone(reactor),
        })
    }

    fn poll_io<T>(
        &self,
        cx: &mut Context<'_>,
        direction: Direction,
        mut operation: impl FnMut(&TcpStream) -> io::Result<T>,
    ) -> Poll<io::Result<T>> {
        loop {
            match operation(&self.stream) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.reactor.wait_for(self.key, direction, cx.waker());
                    return Poll::Pending;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                result => return Poll::Ready(result),
            }
        }
    }

    pub(crate) fn poll_read_with<T>(
        &self,
        cx: &mut Context<'_>,
        operation: impl FnMut(&TcpStream) -> io::Result<T>,
    ) -> Poll<io::Result<T>> {
        self.poll_io(cx, Direction::Read, operation)
    }

    pub(crate) fn poll_write_with<T>(
        &self,
        cx: &mut Context<'_>,
        operation: impl FnMut(&TcpStream) -> io::Result<T>,
    ) -> Poll<io::Result<T>> {
        self.poll_io(cx, Direction::Write, operation)
    }

    pub(crate) fn shutdown(&self) -> io::Result<()> {
        match self.stream.shutdown(std::net::Shutdown::Write) {
            Err(error) if error.kind() != io::ErrorKind::NotConnected => Err(error),
            _ => Ok(()),
        }
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.reactor.forget(self.key, self.stream.as_fd());
    }
}
