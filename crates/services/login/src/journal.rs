use std::io;

const STDOUT_SOCKET: &[u8] = b"/run/systemd/journal/stdout";
const INFO: u8 = 6;

/// A connection to the journal's stream socket, prepared before a fork so that the child can
/// make it after taking the session's user: the journal then logs what the session writes as
/// that user's, in that user's session.
pub struct JournalStream {
    address: libc::sockaddr_un,
    header: Vec<u8>,
}

fn checked(result: libc::c_int) -> io::Result<libc::c_int> {
    match result {
        -1 => Err(io::Error::last_os_error()),
        result => Ok(result),
    }
}

impl JournalStream {
    pub fn new(identifier: &str) -> Self {
        // SAFETY: sockaddr_un is plain data, for which all zeroes is valid.
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (slot, byte) in address.sun_path.iter_mut().zip(STDOUT_SOCKET) {
            *slot = *byte as libc::c_char;
        }
        let header = format!("{identifier}\n\n{INFO}\n0\n0\n0\n0\n").into_bytes();
        Self { address, header }
    }

    /// Points standard output and standard error at the journal. Makes only async-signal-safe
    /// system calls, so a forked child can call it before exec; on failure they stay as they were.
    pub fn attach(&self) -> io::Result<()> {
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
