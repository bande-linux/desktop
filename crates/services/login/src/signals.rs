use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{FromRawFd, OwnedFd};

const STOPPING: [libc::c_int; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP];

/// Blocks the signals that ask this process to stop and returns a descriptor that becomes
/// readable when one arrives, so the process can end in order instead of dying at once.
pub fn stopping() -> io::Result<OwnedFd> {
    // SAFETY: the set is initialised by sigemptyset before use, and signalfd returns a new
    // descriptor this function owns.
    unsafe {
        let mut set = MaybeUninit::<libc::sigset_t>::uninit();
        libc::sigemptyset(set.as_mut_ptr());
        for signal in STOPPING {
            libc::sigaddset(set.as_mut_ptr(), signal);
        }
        let set = set.assume_init();
        if libc::sigprocmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) == -1 {
            return Err(io::Error::last_os_error());
        }
        match libc::signalfd(-1, &set, libc::SFD_CLOEXEC | libc::SFD_NONBLOCK) {
            -1 => Err(io::Error::last_os_error()),
            fd => Ok(OwnedFd::from_raw_fd(fd)),
        }
    }
}

/// Consumes the signal waiting on `signals`, answering whether there was one.
pub fn arrived(signals: &OwnedFd) -> io::Result<bool> {
    let mut info = [0u8; std::mem::size_of::<libc::signalfd_siginfo>()];
    match rustix::io::read(signals, &mut info) {
        Ok(_) => Ok(true),
        Err(rustix::io::Errno::AGAIN) => Ok(false),
        Err(error) => Err(error.into()),
    }
}
