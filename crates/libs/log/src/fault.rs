use std::fs::{self, DirBuilder, File};
use std::io::{self, BufRead, BufReader, Write};
use std::os::fd::OwnedFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use rustix::time::{
    ClockId, Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, clock_gettime,
    timerfd_create, timerfd_settime,
};

use crate::ring::Ring;

const SOCKET: &str = "/run/bande/report.sock";
const ACTIONS: &str = "actions";
const PRIVATE_FOLDER: u32 = 0o700;
const PRIVATE_FILE: u32 = 0o600;
const WAIT_FOR_ANSWER: Duration = Duration::from_secs(15);

fn escaped(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c == '"' || c == '\\' || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect()
}

fn send(request: String, wait: Option<Duration>) -> io::Result<()> {
    let mut stream = UnixStream::connect(SOCKET)?;
    stream.write_all(request.as_bytes())?;
    stream.write_all(b"\n")?;
    if let Some(wait) = wait {
        stream.set_read_timeout(Some(wait))?;
        let mut answer = String::new();
        BufReader::new(stream).read_line(&mut answer)?;
    }
    Ok(())
}

fn send_in_background(request: String) {
    let sent = thread::Builder::new()
        .name("fault-report".to_owned())
        .spawn(move || {
            if let Err(error) = send(request, None) {
                tracing::debug!("cannot leave a fault report: {error}");
            }
        });
    if let Err(error) = sent {
        tracing::debug!("cannot start a fault report: {error}");
    }
}

pub fn report(kind: &str, reason: &str) {
    send_in_background(format!(
        r#"{{"op":"fault","kind":"{}","reason":"{}","pid":{}}}"#,
        escaped(kind),
        escaped(reason),
        std::process::id(),
    ));
}

pub fn report_hang_of(kind: &str, pid: u32, seconds: u64) {
    send_in_background(hang_request(kind, pid, seconds));
}

fn hang_request(kind: &str, pid: u32, seconds: u64) -> String {
    format!(
        r#"{{"op":"fault","kind":"{}","pid":{pid},"seconds":{seconds}}}"#,
        escaped(kind)
    )
}

pub fn report_hang_and_wait(kind: &str, pid: u32, seconds: u64, wait: Duration) {
    if let Err(error) = send(hang_request(kind, pid, seconds), Some(wait)) {
        tracing::debug!("cannot leave a hang report: {error}");
    }
}

pub fn count(name: &str) {
    send_in_background(format!(r#"{{"op":"count","name":"{}"}}"#, escaped(name)));
}

pub fn action(name: &str) {
    static RING: OnceLock<Option<Ring>> = OnceLock::new();
    if let Some(ring) = RING.get_or_init(|| Ring::open(ACTIONS).ok()) {
        ring.note(name);
    }
}

pub fn write_session_facts(json: &str) {
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") else {
        return;
    };
    let folder = PathBuf::from(runtime).join("bande");
    let staged = folder.join(".session.json.new");
    let written = DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_FOLDER)
        .create(&folder)
        .and_then(|()| {
            File::options()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(PRIVATE_FILE)
                .open(&staged)
        })
        .and_then(|mut file| file.write_all(json.as_bytes()))
        .and_then(|()| fs::rename(&staged, folder.join("session.json")));
    if let Err(error) = written {
        tracing::debug!("cannot note the session's facts: {error}");
    }
}

struct StallShared {
    busy_since: AtomicU64,
    generation: AtomicU64,
    armed: AtomicBool,
    timer: OwnedFd,
}

pub struct StallWatch(Arc<StallShared>);

fn now_ns() -> u64 {
    let now = clock_gettime(ClockId::Monotonic);
    (now.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(now.tv_nsec as u64)
}

fn arm(timer: &OwnedFd, after: Duration) -> io::Result<()> {
    let value = Itimerspec {
        it_interval: Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        },
        it_value: Timespec {
            tv_sec: after.as_secs() as i64,
            tv_nsec: i64::from(after.subsec_nanos().max(1)),
        },
    };
    timerfd_settime(timer, TimerfdTimerFlags::empty(), &value)?;
    Ok(())
}

impl StallWatch {
    pub fn start(kind: &'static str, limit: Duration) -> io::Result<Self> {
        let timer = timerfd_create(TimerfdClockId::Monotonic, TimerfdFlags::CLOEXEC)?;
        let shared = Arc::new(StallShared {
            busy_since: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            armed: AtomicBool::new(false),
            timer,
        });
        let watched = Arc::clone(&shared);
        thread::Builder::new()
            .name("stall-watch".to_owned())
            .spawn(move || watch(&watched, kind, limit))?;
        Ok(Self(shared))
    }

    pub fn busy(&self, limit: Duration) {
        self.0.busy_since.store(now_ns(), Ordering::Release);
        self.0.generation.fetch_add(1, Ordering::Relaxed);
        if !self.0.armed.swap(true, Ordering::AcqRel)
            && let Err(error) = arm(&self.0.timer, limit)
        {
            tracing::debug!("cannot arm the stall watch: {error}");
        }
    }

    pub fn idle(&self) {
        self.0.busy_since.store(0, Ordering::Release);
    }
}

fn watch(shared: &StallShared, kind: &'static str, limit: Duration) {
    let mut expirations = [0u8; 8];
    let mut reported = None;
    loop {
        if let Err(error) = rustix::io::read(&shared.timer, &mut expirations) {
            if error == rustix::io::Errno::INTR {
                continue;
            }
            return;
        }
        let since = shared.busy_since.load(Ordering::Acquire);
        if since == 0 {
            shared.armed.store(false, Ordering::Release);
            if shared.busy_since.load(Ordering::Acquire) == 0
                || shared.armed.swap(true, Ordering::AcqRel)
            {
                continue;
            }
            if let Err(error) = arm(&shared.timer, limit) {
                tracing::debug!("cannot arm the stall watch: {error}");
            }
            continue;
        }
        let elapsed = Duration::from_nanos(now_ns().saturating_sub(since));
        let generation = shared.generation.load(Ordering::Relaxed);
        if elapsed >= limit && reported != Some(generation) {
            reported = Some(generation);
            report_hang_and_wait(kind, std::process::id(), elapsed.as_secs(), WAIT_FOR_ANSWER);
        }
        let next = if elapsed >= limit {
            limit
        } else {
            limit - elapsed
        };
        if let Err(error) = arm(&shared.timer, next) {
            tracing::debug!("cannot arm the stall watch: {error}");
        }
    }
}
