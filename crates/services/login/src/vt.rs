use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use bande_design::{Palette, Scheme};
use libc::{c_char, c_int, c_short, c_ulong, c_ushort};

pub const GREETER_VT: u32 = 1;
const CONSOLE: &str = "/dev/tty0";
const VT_OPENQRY: c_ulong = 0x5600;
const VT_SETMODE: c_ulong = 0x5602;
const VT_GETSTATE: c_ulong = 0x5603;
const VT_RELDISP: c_ulong = 0x5605;
const VT_ACTIVATE: c_ulong = 0x5606;
const KDSETMODE: c_ulong = 0x4B3A;
const KD_TEXT: c_int = 0;
const KD_GRAPHICS: c_int = 1;
const VT_AUTO: c_char = 0;
const VT_PROCESS: c_char = 1;
const VT_ACKACQ: c_int = 2;
const SWITCH_LIMIT: Duration = Duration::from_secs(3);
const SWITCH_POLL: Duration = Duration::from_millis(10);

#[repr(C)]
struct VtMode {
    mode: c_char,
    waitv: c_char,
    relsig: c_short,
    acqsig: c_short,
    frsig: c_short,
}

#[repr(C)]
#[derive(Default)]
struct VtStat {
    active: c_ushort,
    signal: c_ushort,
    state: c_ushort,
}

static HELD: AtomicI32 = AtomicI32::new(-1);

fn checked(result: c_int) -> io::Result<()> {
    match result {
        -1 => Err(io::Error::last_os_error()),
        _ => Ok(()),
    }
}

fn console() -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY | libc::O_CLOEXEC)
        .open(CONSOLE)
}

/// The virtual terminal on screen.
pub fn active() -> io::Result<u32> {
    let console = console()?;
    let mut state = VtStat::default();
    // SAFETY: VT_GETSTATE fills a struct vt_stat, which VtStat matches field for field.
    checked(unsafe { libc::ioctl(console.as_raw_fd(), VT_GETSTATE, &mut state) })?;
    Ok(u32::from(state.active))
}

/// Asks the kernel to bring `number` to the front, as Ctrl+Alt+F`number` does.
pub fn activate(number: u32) -> io::Result<()> {
    let console = console()?;
    let number = c_int::try_from(number).map_err(io::Error::other)?;
    // SAFETY: VT_ACTIVATE takes the terminal's number as a plain integer.
    checked(unsafe { libc::ioctl(console.as_raw_fd(), VT_ACTIVATE, number) })
}

/// Brings `number` to the front and waits until it is there.
pub fn switch_to(number: u32) -> io::Result<()> {
    if active()? == number {
        return Ok(());
    }
    activate(number)?;
    let deadline = Instant::now() + SWITCH_LIMIT;
    while active()? != number {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("virtual terminal {number} did not come to the front"),
            ));
        }
        thread::sleep(SWITCH_POLL);
    }
    Ok(())
}

/// A virtual terminal no one has open.
pub fn unused() -> io::Result<u32> {
    let console = console()?;
    let mut number: c_int = -1;
    // SAFETY: VT_OPENQRY writes the first free terminal's number to the int it points to.
    checked(unsafe { libc::ioctl(console.as_raw_fd(), VT_OPENQRY, &mut number) })?;
    u32::try_from(number)
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| io::Error::other("every virtual terminal is in use"))
}

extern "C" fn released(_: c_int) {
    // SAFETY: VT_RELDISP takes a plain integer; ioctl is async-signal-safe and the fd stays open
    // while it is held.
    unsafe { libc::ioctl(HELD.load(Ordering::Relaxed), VT_RELDISP, 1) };
}

extern "C" fn acquired(_: c_int) {
    // SAFETY: as in `released`.
    unsafe { libc::ioctl(HELD.load(Ordering::Relaxed), VT_RELDISP, VT_ACKACQ) };
}

fn handle(signal: c_int, handler: extern "C" fn(c_int)) -> io::Result<()> {
    // SAFETY: a zeroed sigaction with only the handler and SA_RESTART set is valid; the handler
    // makes one async-signal-safe call.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler as libc::sighandler_t;
        action.sa_flags = libc::SA_RESTART;
        checked(libc::sigemptyset(&mut action.sa_mask))?;
        checked(libc::sigaction(signal, &action, std::ptr::null_mut()))
    }
}

/// A session's virtual terminal, kept open while the session runs so no one else takes it.
pub struct Terminal {
    file: File,
    number: u32,
    held: bool,
}

impl Terminal {
    pub fn open(number: u32) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY | libc::O_CLOEXEC)
            .open(format!("/dev/tty{number}"))?;
        Ok(Self {
            file,
            number,
            held: false,
        })
    }

    pub fn number(&self) -> u32 {
        self.number
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }

    /// Empties the console and paints it in the login window's background with no cursor, so
    /// that whenever the kernel shows it, it shows only that colour.
    pub fn blank(&mut self) {
        let deep = Palette::for_scheme(Scheme::Dark).deep;
        let blank = format!(
            "\x1b[?25l\x1b]P0{:02x}{:02x}{:02x}\x1b[H\x1b[2J",
            deep.r, deep.g, deep.b
        );
        if let Err(error) = self.file.write_all(blank.as_bytes()) {
            tracing::warn!("tty{} may show text: {error}", self.number);
        }
    }

    fn ioctl(&self, request: c_ulong, value: c_int) -> io::Result<()> {
        // SAFETY: every request passed here takes a plain integer, not a pointer.
        checked(unsafe { libc::ioctl(self.file.as_raw_fd(), request, value) })
    }

    fn set_mode(&self, mode: &VtMode) -> io::Result<()> {
        // SAFETY: VT_SETMODE reads a struct vt_mode, which VtMode matches field for field.
        checked(unsafe { libc::ioctl(self.file.as_raw_fd(), VT_SETMODE, mode) })
    }

    /// Brings the terminal to the front in graphics mode, so the kernel's console never draws
    /// over the frame on screen, acknowledging every switch until the session's compositor
    /// takes the terminal over. A terminal already on screen is left as it is.
    pub fn show(&mut self) -> io::Result<()> {
        if active()? == self.number {
            return Ok(());
        }
        let fd: RawFd = self.file.as_raw_fd();
        HELD.store(fd, Ordering::Relaxed);
        handle(libc::SIGUSR1, released)?;
        handle(libc::SIGUSR2, acquired)?;
        let signal = |number: c_int| c_short::try_from(number).map_err(io::Error::other);
        self.set_mode(&VtMode {
            mode: VT_PROCESS,
            waitv: 0,
            relsig: signal(libc::SIGUSR1)?,
            acqsig: signal(libc::SIGUSR2)?,
            frsig: 0,
        })?;
        self.held = true;
        self.ioctl(KDSETMODE, KD_GRAPHICS)?;
        switch_to(self.number)
    }

    fn restore(&self) -> io::Result<()> {
        self.ioctl(KDSETMODE, KD_TEXT)?;
        self.set_mode(&VtMode {
            mode: VT_AUTO,
            waitv: 0,
            relsig: 0,
            acqsig: 0,
            frsig: 0,
        })
    }

    /// Gives the terminal back to the kernel's console whoever left it in graphics mode, resets
    /// it and brings it to the front, so a login prompt there can be seen.
    pub fn give_to_console(&self) -> io::Result<()> {
        self.restore()?;
        (&self.file).write_all(b"\x1bc")?;
        switch_to(self.number)
    }

    /// Hands the screen to `other` if this terminal still has it.
    pub fn leave_to(&self, other: u32) -> io::Result<()> {
        match active()? == self.number {
            true => switch_to(other),
            false => Ok(()),
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if !self.held {
            return;
        }
        if let Err(error) = self.restore() {
            tracing::warn!(
                "cannot give tty{} back to the console: {error}",
                self.number
            );
        }
        HELD.store(-1, Ordering::Relaxed);
    }
}
