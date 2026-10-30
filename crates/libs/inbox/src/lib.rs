use std::io::{self, PipeReader, PipeWriter, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

pub struct Inbox<T> {
    receiver: Receiver<T>,
    reader: PipeReader,
    outbox: Outbox<T>,
}

pub struct Outbox<T> {
    sender: Sender<T>,
    writer: Arc<PipeWriter>,
}

impl<T> Clone for Outbox<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            writer: Arc::clone(&self.writer),
        }
    }
}

impl<T> Outbox<T> {
    pub fn send(&self, value: T) -> bool {
        self.sender.send(value).is_ok() && (&*self.writer).write_all(&[1]).is_ok()
    }
}

impl<T: Clone + PartialEq + Send + 'static> Outbox<T> {
    pub fn watch(self, every: Duration, mut probe: impl FnMut() -> T + Send + 'static) {
        thread::spawn(move || {
            let mut last = None;
            loop {
                let value = probe();
                if last.as_ref() != Some(&value) {
                    if !self.send(value.clone()) {
                        return;
                    }
                    last = Some(value);
                }
                thread::sleep(every);
            }
        });
    }
}

impl<T> Inbox<T> {
    pub fn new() -> io::Result<Self> {
        let (reader, writer) = io::pipe()?;
        rustix::fs::fcntl_setfl(&reader, rustix::fs::OFlags::NONBLOCK)?;
        let (sender, receiver) = mpsc::channel();
        Ok(Self {
            receiver,
            reader,
            outbox: Outbox {
                sender,
                writer: Arc::new(writer),
            },
        })
    }

    pub fn outbox(&self) -> Outbox<T> {
        self.outbox.clone()
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.reader.as_fd()
    }

    pub fn drain(&mut self) -> Vec<T> {
        let mut bytes = [0; 64];
        while matches!((&self.reader).read(&mut bytes), Ok(read) if read > 0) {}
        self.receiver.try_iter().collect()
    }
}
