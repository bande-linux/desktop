use std::fs::{self, DirBuilder, File};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

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

#[derive(Default)]
struct StallState {
    busy_since: Option<Instant>,
    generation: u64,
    watching: bool,
}

#[derive(Default)]
struct StallShared {
    state: Mutex<StallState>,
    wake: Condvar,
}

pub struct StallWatch(Arc<StallShared>);

impl StallWatch {
    pub fn start(kind: &'static str, limit: Duration) -> io::Result<Self> {
        let shared = Arc::new(StallShared::default());
        let watched = Arc::clone(&shared);
        thread::Builder::new()
            .name("stall-watch".to_owned())
            .spawn(move || watch(&watched, kind, limit))?;
        Ok(Self(shared))
    }

    pub fn busy(&self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.busy_since = Some(Instant::now());
            state.generation = state.generation.wrapping_add(1);
            if !state.watching {
                state.watching = true;
                self.0.wake.notify_one();
            }
        }
    }

    pub fn idle(&self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.busy_since = None;
        }
    }
}

fn watch(shared: &StallShared, kind: &'static str, limit: Duration) {
    let mut reported = None;
    let Ok(mut state) = shared.state.lock() else {
        return;
    };
    loop {
        let Some(since) = state.busy_since else {
            state.watching = false;
            let Ok(next) = shared.wake.wait(state) else {
                return;
            };
            state = next;
            continue;
        };
        let elapsed = since.elapsed();
        if elapsed < limit {
            let Ok((next, _)) = shared.wake.wait_timeout(state, limit - elapsed) else {
                return;
            };
            state = next;
            continue;
        }
        if reported != Some(state.generation) {
            reported = Some(state.generation);
            let seconds = elapsed.as_secs();
            drop(state);
            report_hang_and_wait(kind, std::process::id(), seconds, WAIT_FOR_ANSWER);
            let Ok(next) = shared.state.lock() else {
                return;
            };
            state = next;
            continue;
        }
        let Ok((next, _)) = shared.wake.wait_timeout(state, limit) else {
            return;
        };
        state = next;
    }
}
