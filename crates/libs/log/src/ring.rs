use std::fmt::Write as _;
use std::fs::{DirBuilder, File};
use std::io;
use std::os::fd::AsFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use rustix::fs::{FallocateFlags, OFlags, fallocate};
use rustix::mm::{MapFlags, ProtFlags, mmap};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

const HEADER: usize = 64;
const TEXT: usize = 32 * 1024;
const LONGEST_LINE: usize = 512;
const DIRECTORY_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
const RUNTIME_DIRECTORY: &str = "XDG_RUNTIME_DIR";

pub struct Ring {
    position: &'static AtomicU64,
    text: &'static [AtomicU8],
}

fn monotonic_ms() -> i64 {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    now.tv_sec * 1000 + now.tv_nsec / 1_000_000
}

fn folder() -> Option<PathBuf> {
    let runtime = std::env::var_os(RUNTIME_DIRECTORY)?;
    Some(PathBuf::from(runtime).join("bande/ring"))
}

impl Ring {
    pub fn open(name: &str) -> io::Result<Self> {
        let folder = folder().ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        DirBuilder::new()
            .recursive(true)
            .mode(DIRECTORY_MODE)
            .create(&folder)?;
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(FILE_MODE)
            .custom_flags((OFlags::NOFOLLOW | OFlags::CLOEXEC).bits() as i32)
            .open(folder.join(name))?;
        let size = HEADER + TEXT;
        fallocate(file.as_fd(), FallocateFlags::empty(), 0, size as u64)?;
        let mapped = unsafe {
            mmap(
                null_mut(),
                size,
                ProtFlags::READ | ProtFlags::WRITE,
                MapFlags::SHARED,
                file.as_fd(),
                0,
            )?
        };
        let (position, text) = unsafe {
            (
                &*mapped.cast::<AtomicU64>(),
                std::slice::from_raw_parts(
                    mapped.cast::<u8>().add(HEADER).cast::<AtomicU8>(),
                    TEXT,
                ),
            )
        };
        Ok(Self { position, text })
    }

    pub fn write_line(&self, line: &str) {
        let bytes = line.as_bytes();
        let start = self
            .position
            .fetch_add(bytes.len() as u64, Ordering::Relaxed) as usize;
        for (index, byte) in bytes.iter().enumerate() {
            self.text[(start + index) % TEXT].store(*byte, Ordering::Relaxed);
        }
    }

    pub fn note(&self, text: &str) {
        let mut line = format!("{} {} ", std::process::id(), monotonic_ms());
        line.extend(
            text.chars()
                .take(LONGEST_LINE)
                .map(|c| if c == '\n' { ' ' } else { c }),
        );
        line.push('\n');
        self.write_line(&line);
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name().starts_with("log.") {
            return;
        }
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?} ");
        } else {
            let _ = write!(self.0, "{}={value:?} ", field.name());
        }
    }
}

pub struct RingLayer(pub Ring);

impl<S: Subscriber> Layer<S> for RingLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut message = Message::default();
        event.record(&mut message);
        let metadata = event.metadata();
        self.0.note(&format!(
            "{} {}: {}",
            metadata.level(),
            metadata.target(),
            message.0
        ));
    }
}
