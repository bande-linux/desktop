use std::io;
use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicI32, Ordering};

use crate::checked;

const STOPPING: [libc::c_int; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP];

static NOTICE: AtomicI32 = AtomicI32::new(-1);

/// Runs `call`, which a signal handler may make, and puts the interrupted code's errno back.
pub fn keeping_errno(call: impl FnOnce()) {
    // SAFETY: errno's location is valid for the life of the thread.
    let saved = unsafe { *libc::__errno_location() };
    call();
    // SAFETY: as above.
    unsafe { *libc::__errno_location() = saved };
}

extern "C" fn noticed(_: libc::c_int) {
    keeping_errno(|| {
        // SAFETY: write is async-signal-safe; NOTICE is the pipe's write end, which stays open
        // for the life of the process, and the byte lives across the call.
        unsafe { libc::write(NOTICE.load(Ordering::Relaxed), [1u8].as_ptr().cast(), 1) };
    });
}

/// Catches the signals that ask this process to stop and returns a descriptor that becomes
/// readable when one arrives, so the process can end in order instead of dying at once. The
/// signals are handled, not blocked, so the programs this process starts (PAM's helpers, the
/// session) get them as usual.
pub fn stopping() -> io::Result<OwnedFd> {
    let mut ends = [0; 2];
    // SAFETY: pipe2 writes two new descriptors into ends.
    checked(unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) })?;
    // SAFETY: pipe2 just opened both descriptors and nothing else owns the read end.
    let notices = unsafe { OwnedFd::from_raw_fd(ends[0]) };
    NOTICE.store(ends[1], Ordering::Relaxed);
    for signal in STOPPING {
        handle(signal, noticed)?;
    }
    Ok(notices)
}

/// Runs `handler` on `signal`, restarting interrupted system calls.
pub fn handle(signal: libc::c_int, handler: extern "C" fn(libc::c_int)) -> io::Result<()> {
    // SAFETY: a zeroed sigaction with only the handler and SA_RESTART set is valid; every
    // handler passed here makes only async-signal-safe calls.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler as libc::sighandler_t;
        action.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut action.sa_mask);
        checked(libc::sigaction(signal, &action, std::ptr::null_mut()))?;
    }
    Ok(())
}

/// Consumes the signals waiting on `notices`, answering whether there were any.
pub fn arrived(notices: &OwnedFd) -> io::Result<bool> {
    let mut buffer = [0u8; 64];
    let mut any = false;
    loop {
        match rustix::io::read(notices, &mut buffer) {
            Ok(0) | Err(rustix::io::Errno::AGAIN) => return Ok(any),
            Ok(_) => any = true,
            Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(error.into()),
        }
    }
}
