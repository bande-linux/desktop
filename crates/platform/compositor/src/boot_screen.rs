use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::inotify::{self, CreateFlags, WatchFlags};
use rustix::ioctl::{self, IntegerSetter, Opcode};

pub(crate) const PLYMOUTH_PID: &str = "/run/plymouth/pid";
const HANDOVER_FOLDER: &str = "/run/bande-greeter";
const HANDOVER_WANTED: &str = "/run/bande-greeter/handover";
const HANDED_OVER: &str = "/run/bande-greeter/handed-over";
const HANDOVER_LIMIT: Duration = Duration::from_secs(3);
const KDSETMODE: Opcode = 0x4B3A;
const KD_GRAPHICS: usize = 1;

fn hide_console_text() -> io::Result<()> {
    let terminal = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY | libc::O_CLOEXEC)
        .open("/dev/tty")?;
    // SAFETY: KDSETMODE takes the console mode as a plain integer, not a pointer.
    let graphics = unsafe { IntegerSetter::<KDSETMODE>::new_usize(KD_GRAPHICS) };
    // SAFETY: terminal is this process's open controlling terminal, and the request carries no pointer.
    unsafe { ioctl::ioctl(terminal.as_fd(), graphics) }?;
    Ok(())
}

fn ask_for_display() -> io::Result<bool> {
    let watch = inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)?;
    inotify::add_watch(
        &watch,
        HANDOVER_FOLDER,
        WatchFlags::CREATE | WatchFlags::MOVED_TO,
    )?;
    std::fs::write(HANDOVER_WANTED, b"")?;
    let deadline = Instant::now() + HANDOVER_LIMIT;
    let mut events = [0; 4096];
    while !Path::new(HANDED_OVER).exists() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(false);
        }
        let timeout = Timespec::try_from(left).map_err(io::Error::other)?;
        poll(&mut [PollFd::new(&watch, PollFlags::IN)], Some(&timeout))?;
        match rustix::io::read(&watch, &mut events) {
            Ok(_) | Err(rustix::io::Errno::AGAIN) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(true)
}

/// Takes the display over from Plymouth while the boot screen still shows: keeps the kernel's
/// console from drawing, then has the root `bande-greeter-handover.path` run `plymouth
/// deactivate`, waiting at most a few seconds for it.
pub fn take_display_from_boot_screen() {
    if !Path::new(PLYMOUTH_PID).exists() || Path::new(HANDED_OVER).exists() {
        return;
    }
    if let Err(error) = hide_console_text() {
        tracing::warn!("the console may show before the login window: {error}");
    }
    match ask_for_display() {
        Ok(true) => tracing::info!("the boot screen gave up the display"),
        Ok(false) => tracing::warn!(
            "the boot screen did not hand over the display within {} s \
             (see bande-greeter-handover.service); taking it anyway",
            HANDOVER_LIMIT.as_secs()
        ),
        Err(error) => tracing::warn!("cannot ask the boot screen for the display: {error}"),
    }
}
