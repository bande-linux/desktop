use std::io::{self, IoSlice, IoSliceMut};
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use rustix::cmsg_space;
use rustix::net::{
    AddressFamily, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags,
    SendAncillaryBuffer, SendAncillaryMessage, SendFlags, SocketFlags, SocketType, recvmsg,
    sendmsg, socketpair,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

const MESSAGE_LIMIT: usize = 64 * 1024;
const FD_LIMIT: usize = 4;

pub struct Channel {
    socket: OwnedFd,
}

impl Channel {
    pub fn pair() -> io::Result<(Self, Self)> {
        let (first, second) = socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )?;
        Ok((Self { socket: first }, Self { socket: second }))
    }

    pub fn from_fd(socket: OwnedFd) -> Self {
        Self { socket }
    }

    pub fn into_fd(self) -> OwnedFd {
        self.socket
    }

    pub fn send<T: Serialize>(
        &self,
        message: &T,
        fds: &[BorrowedFd<'_>],
        flags: SendFlags,
    ) -> io::Result<()> {
        let bytes = serde_json::to_vec(message).map_err(io::Error::other)?;
        if bytes.len() > MESSAGE_LIMIT || fds.len() > FD_LIMIT {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let mut space = [MaybeUninit::uninit(); cmsg_space!(ScmRights(FD_LIMIT))];
        let mut ancillary = SendAncillaryBuffer::new(&mut space);
        if !fds.is_empty() && !ancillary.push(SendAncillaryMessage::ScmRights(fds)) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        sendmsg(
            &self.socket,
            &[IoSlice::new(&bytes)],
            &mut ancillary,
            flags | SendFlags::NOSIGNAL,
        )?;
        Ok(())
    }

    pub fn receive<T: DeserializeOwned>(
        &self,
        flags: RecvFlags,
    ) -> io::Result<Option<(T, Vec<OwnedFd>)>> {
        let mut bytes = vec![0; MESSAGE_LIMIT];
        let mut space = [MaybeUninit::uninit(); cmsg_space!(ScmRights(FD_LIMIT))];
        let mut ancillary = RecvAncillaryBuffer::new(&mut space);
        let received = recvmsg(
            &self.socket,
            &mut [IoSliceMut::new(&mut bytes)],
            &mut ancillary,
            flags | RecvFlags::CMSG_CLOEXEC,
        )?;
        if received.bytes == 0 {
            return Ok(None);
        }
        if received.flags.contains(ReturnFlags::TRUNC) {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let mut fds = Vec::new();
        for message in ancillary.drain() {
            if let RecvAncillaryMessage::ScmRights(rights) = message {
                fds.extend(rights);
            }
        }
        let message = serde_json::from_slice(&bytes[..received.bytes])
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        Ok(Some((message, fds)))
    }
}

impl AsFd for Channel {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.socket.as_fd()
    }
}
