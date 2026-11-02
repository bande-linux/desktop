use std::collections::VecDeque;
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use rustix::io::{Errno, IoSlice, IoSliceMut};
use rustix::net::{
    AddressFamily, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags,
    SendAncillaryBuffer, SendAncillaryMessage, SendFlags, SocketFlags, SocketType,
};

use crate::wire::{Reader, Wire, Writer};

pub const MAX_MESSAGE: usize = 64 << 10;
pub const MAX_FDS: usize = 4;
const MAX_QUEUED: usize = 4096;

pub struct Message {
    pub bytes: Vec<u8>,
    pub fds: Vec<OwnedFd>,
}

impl Message {
    pub fn decode<T: Wire>(&self) -> io::Result<T> {
        let mut reader = Reader::new(&self.bytes);
        let value = reader.get()?;
        reader.finish()?;
        Ok(value)
    }

    pub fn fd(&mut self) -> io::Result<OwnedFd> {
        match self.fds.pop() {
            Some(fd) if self.fds.is_empty() => Ok(fd),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a message without its descriptor",
            )),
        }
    }
}

pub struct Channel {
    socket: OwnedFd,
    queue: VecDeque<(Vec<u8>, Vec<OwnedFd>)>,
    buffer: Vec<u8>,
}

impl Channel {
    pub fn pair() -> io::Result<(Channel, Channel)> {
        let (one, two) = rustix::net::socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            None,
        )?;
        Ok((Channel::new(one)?, Channel::new(two)?))
    }

    pub fn new(socket: OwnedFd) -> io::Result<Channel> {
        let flags = rustix::fs::fcntl_getfl(&socket)?;
        rustix::fs::fcntl_setfl(&socket, flags | rustix::fs::OFlags::NONBLOCK)?;
        Ok(Channel {
            socket,
            queue: VecDeque::new(),
            buffer: Vec::new(),
        })
    }

    pub fn into_fd(self) -> OwnedFd {
        self.socket
    }

    pub fn wants_write(&self) -> bool {
        !self.queue.is_empty()
    }

    pub fn send<T: Wire>(&mut self, message: &T, fds: Vec<OwnedFd>) -> io::Result<()> {
        let mut writer = Writer::default();
        writer.put(message);
        let bytes = writer.into_bytes();
        if bytes.len() > MAX_MESSAGE || fds.len() > MAX_FDS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a message over the size limit",
            ));
        }
        if self.queue.len() >= MAX_QUEUED {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "the peer stopped reading",
            ));
        }
        self.queue.push_back((bytes, fds));
        self.flush()
    }

    pub fn flush(&mut self) -> io::Result<()> {
        while let Some((bytes, fds)) = self.queue.front() {
            let borrowed: Vec<BorrowedFd<'_>> = fds.iter().map(AsFd::as_fd).collect();
            let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_FDS))];
            let mut control = SendAncillaryBuffer::new(&mut space);
            if !borrowed.is_empty() && !control.push(SendAncillaryMessage::ScmRights(&borrowed)) {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            let sent = rustix::net::sendmsg(
                &self.socket,
                &[IoSlice::new(bytes)],
                &mut control,
                SendFlags::NOSIGNAL,
            );
            match sent {
                Ok(_) => {
                    self.queue.pop_front();
                }
                Err(Errno::INTR) => {}
                Err(Errno::AGAIN) => return Ok(()),
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    pub fn receive(&mut self) -> io::Result<Option<Message>> {
        self.buffer.resize(MAX_MESSAGE, 0);
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_FDS))];
        let mut control = RecvAncillaryBuffer::new(&mut space);
        let received = loop {
            match rustix::net::recvmsg(
                &self.socket,
                &mut [IoSliceMut::new(&mut self.buffer)],
                &mut control,
                RecvFlags::CMSG_CLOEXEC,
            ) {
                Err(Errno::INTR) => {}
                other => break other,
            }
        };
        let received = match received {
            Ok(received) => received,
            Err(Errno::AGAIN) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut fds = Vec::new();
        for message in control.drain() {
            if let RecvAncillaryMessage::ScmRights(rights) = message {
                fds.extend(rights);
            }
        }
        if received
            .flags
            .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a message over the size limit",
            ));
        }
        if received.bytes == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let bytes = self
            .buffer
            .get(..received.bytes)
            .unwrap_or_default()
            .to_vec();
        Ok(Some(Message { bytes, fds }))
    }
}

impl AsFd for Channel {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.socket.as_fd()
    }
}
