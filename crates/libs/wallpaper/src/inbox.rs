use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

pub struct Inbox<T> {
    receiver: Receiver<T>,
    wake: UnixStream,
}

pub struct Outbox<T> {
    sender: Sender<T>,
    wake: Arc<UnixStream>,
}

impl<T> Clone for Outbox<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            wake: Arc::clone(&self.wake),
        }
    }
}

pub fn inbox<T>() -> io::Result<(Inbox<T>, Outbox<T>)> {
    let (reader, writer) = UnixStream::pair()?;
    reader.set_nonblocking(true)?;
    writer.set_nonblocking(true)?;
    let (sender, receiver) = mpsc::channel();
    Ok((
        Inbox {
            receiver,
            wake: reader,
        },
        Outbox {
            sender,
            wake: Arc::new(writer),
        },
    ))
}

impl<T> Outbox<T> {
    pub fn send(&self, message: T) -> bool {
        if self.sender.send(message).is_err() {
            return false;
        }
        match (&*self.wake).write_all(&[1]) {
            Ok(()) => true,
            Err(error) => error.kind() == io::ErrorKind::WouldBlock,
        }
    }
}

impl<T> Inbox<T> {
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    pub fn drain(&self) -> Vec<T> {
        let mut bytes = [0; 64];
        while matches!((&self.wake).read(&mut bytes), Ok(read) if read > 0) {}
        self.receiver.try_iter().collect()
    }
}
