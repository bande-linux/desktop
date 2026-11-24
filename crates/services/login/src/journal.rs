use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use crate::checked;

const STDOUT_SOCKET: &[u8] = b"/run/systemd/journal/stdout";
const INFO: u8 = 6;

/// A connection to the journal's stream socket, prepared before a fork so that the child can
/// make it after taking the session's user: the journal then logs what the session writes as
/// that user's, in that user's session.
pub struct JournalStream {
    address: libc::sockaddr_un,
    header: Vec<u8>,
    failures: OwnedFd,
}

/// Where a child that could not reach the journal says why, read by the parent once the child
/// has run its program.
pub struct Failures(OwnedFd);

impl JournalStream {
    pub fn new(identifier: &str) -> io::Result<(Self, Failures)> {
        // SAFETY: sockaddr_un is plain data, for which all zeroes is valid.
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (slot, byte) in address.sun_path.iter_mut().zip(STDOUT_SOCKET) {
            *slot = *byte as libc::c_char;
        }
        let header = format!("{identifier}\n\n{INFO}\n0\n0\n0\n0\n").into_bytes();
        let mut ends = [0; 2];
        // SAFETY: pipe2 writes two new descriptors into ends.
        checked(unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) })?;
        // SAFETY: pipe2 just opened both descriptors, and each is owned once.
        let (told, failures) =
            unsafe { (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1])) };
        Ok((
            Self {
                address,
                header,
                failures,
            },
            Failures(told),
        ))
    }

    /// Points standard output and standard error at the journal, or leaves them as they were
    /// and reports why to [`Failures`]. Makes only async-signal-safe system calls, so a forked
    /// child can call it before exec.
    pub fn attach(&self) {
        if let Err(error) = self.open() {
            let code = error.raw_os_error().unwrap_or(libc::EIO).to_ne_bytes();
            // SAFETY: code is a readable buffer of code.len() bytes and failures is open.
            unsafe { libc::write(self.failures.as_raw_fd(), code.as_ptr().cast(), code.len()) };
        }
    }

    fn open(&self) -> io::Result<()> {
        // SAFETY: socket takes plain integers and returns a descriptor this function owns.
        let fd = checked(unsafe {
            libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0)
        })?;
        let attached = self.connect(fd);
        // SAFETY: fd was opened above and nothing else refers to it.
        unsafe { libc::close(fd) };
        attached
    }

    fn connect(&self, fd: libc::c_int) -> io::Result<()> {
        let length = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
        // SAFETY: address is a valid sockaddr_un of `length` bytes that outlives the call.
        checked(unsafe { libc::connect(fd, (&raw const self.address).cast(), length) })?;
        // SAFETY: shutdown takes plain integers.
        checked(unsafe { libc::shutdown(fd, libc::SHUT_RD) })?;
        let mut rest = self.header.as_slice();
        while !rest.is_empty() {
            // SAFETY: rest points to rest.len() readable bytes.
            let written = checked(
                unsafe { libc::write(fd, rest.as_ptr().cast(), rest.len()) } as libc::c_int
            )?;
            rest = &rest[written as usize..];
        }
        for target in [libc::STDOUT_FILENO, libc::STDERR_FILENO] {
            // SAFETY: dup2 takes plain integers; both descriptors are open.
            checked(unsafe { libc::dup2(fd, target) })?;
        }
        Ok(())
    }
}

impl Failures {
    /// Why the child could not reach the journal, once every copy of the stream is gone: the
    /// parent's (dropped with the command) and the child's (closed by exec).
    pub fn reason(self) -> Option<io::Error> {
        let mut code = [0u8; 4];
        match rustix::io::read(&self.0, &mut code) {
            Ok(4) => Some(io::Error::from_raw_os_error(i32::from_ne_bytes(code))),
            Ok(_) => None,
            Err(error) => Some(error.into()),
        }
    }
}
