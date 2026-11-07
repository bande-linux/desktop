use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use rustix::process::{Pid, Signal, kill_process};
use serde::Deserialize;

const CACHE: &str = "/var/cache/bande-backup";
const FORCED_STOP_AFTER: Duration = Duration::from_secs(30);
const PROGRESS_PER_SECOND: &str = "0.5";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Done,
    SomeFilesUnreadable,
    NoRepository,
    Locked,
    WrongPassword,
    Interrupted,
    Failed,
}

impl Exit {
    fn of(status: ExitStatus) -> Self {
        match status.code() {
            Some(0) => Self::Done,
            Some(3) => Self::SomeFilesUnreadable,
            Some(10) => Self::NoRepository,
            Some(11) => Self::Locked,
            Some(12) => Self::WrongPassword,
            Some(130) | None => Self::Interrupted,
            Some(_) => Self::Failed,
        }
    }

    pub fn saved_a_snapshot(self) -> bool {
        matches!(self, Self::Done | Self::SomeFilesUnreadable)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Paused,
    DiskGone,
    NoSpace,
    OnBattery,
    Reconfigured,
}

#[derive(Debug, Default)]
struct Running {
    pid: Option<u32>,
    reason: Option<StopReason>,
}

#[derive(Debug, Clone, Default)]
pub struct Stopper {
    running: Arc<Mutex<Running>>,
    generation: Arc<AtomicU32>,
}

impl Stopper {
    pub fn stop(&self, reason: StopReason) {
        let pid = {
            let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
            running.reason.get_or_insert(reason);
            running.pid
        };
        let Some(pid) = pid else {
            return;
        };
        signal(pid, Signal::INT);
        let generation = self.generation.load(Ordering::SeqCst);
        let stopper = self.clone();
        thread::spawn(move || {
            thread::sleep(FORCED_STOP_AFTER);
            if stopper.generation.load(Ordering::SeqCst) == generation
                && let Some(pid) = stopper.pid()
            {
                signal(pid, Signal::KILL);
            }
        });
    }

    pub fn take_reason(&self) -> Option<StopReason> {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reason
            .take()
    }

    pub fn is_stopping(&self) -> bool {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reason
            .is_some()
    }

    fn pid(&self) -> Option<u32> {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pid
    }

    fn set_pid(&self, pid: Option<u32>) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let stopping = {
            let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
            running.pid = pid;
            running.reason.is_some()
        };
        if stopping && let Some(pid) = pid {
            signal(pid, Signal::INT);
        }
    }
}

fn signal(pid: u32, signal: Signal) {
    let target = i32::try_from(pid).ok().and_then(Pid::from_raw);
    if let Some(target) = target
        && let Err(error) = kill_process(target, signal)
    {
        tracing::debug!("cannot signal restic: {error}");
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub time: String,
}

impl Snapshot {
    pub fn seconds(&self) -> u64 {
        self.time
            .parse::<jiff::Timestamp>()
            .ok()
            .and_then(|stamp| u64::try_from(stamp.as_second()).ok())
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct Progress {
    #[serde(default)]
    pub percent_done: f64,
    #[serde(default)]
    pub seconds_remaining: u64,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "message_type", rename_all = "snake_case")]
enum Message {
    Status(Progress),
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone)]
pub struct Repository {
    pub path: PathBuf,
    pub password_file: PathBuf,
    wrapper: Vec<String>,
}

pub struct Finished {
    pub exit: Exit,
    pub output: String,
}

impl Repository {
    pub fn new(path: PathBuf, password_file: &Path) -> Self {
        Self {
            path,
            password_file: password_file.to_path_buf(),
            wrapper: Vec::new(),
        }
    }

    pub fn inside(&self, wrapper: Vec<String>) -> Self {
        Self {
            wrapper,
            ..self.clone()
        }
    }

    pub fn process(&self) -> Command {
        let mut command = match self.wrapper.split_first() {
            Some((program, args)) => {
                let mut command = Command::new(program);
                command.args(args).arg("restic");
                command
            }
            None => Command::new("restic"),
        };
        command
            .env("RESTIC_REPOSITORY", &self.path)
            .env("RESTIC_PASSWORD_FILE", &self.password_file)
            .env("RESTIC_CACHE_DIR", CACHE)
            .env("RESTIC_PROGRESS_FPS", PROGRESS_PER_SECOND)
            .stdin(Stdio::null());
        command
    }

    pub fn run<S: AsRef<std::ffi::OsStr>>(
        &self,
        stopper: &Stopper,
        args: &[S],
    ) -> io::Result<Finished> {
        self.follow(stopper, args, |_| {})
    }

    pub fn follow<S: AsRef<std::ffi::OsStr>>(
        &self,
        stopper: &Stopper,
        args: &[S],
        mut on_line: impl FnMut(&str),
    ) -> io::Result<Finished> {
        let mut child = self
            .process()
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        stopper.set_pid(Some(child.id()));
        let errors = child.stderr.take().map(|stderr| {
            thread::spawn(move || {
                let mut collected = String::new();
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    tracing::debug!("restic: {line}");
                    collected.push_str(&line);
                    collected.push('\n');
                }
                collected
            })
        });
        let mut output = String::new();
        if let Some(stdout) = child.stdout.take() {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                on_line(&line);
                if !line.starts_with("{\"message_type\":\"status\"") {
                    output.push_str(&line);
                    output.push('\n');
                }
            }
        }
        let status = child.wait();
        stopper.set_pid(None);
        let status = status?;
        if let Some(errors) = errors
            && let Ok(errors) = errors.join()
        {
            output.push_str(&errors);
        }
        Ok(Finished {
            exit: Exit::of(status),
            output,
        })
    }

    pub fn follow_progress(
        &self,
        stopper: &Stopper,
        args: &[String],
        mut on_progress: impl FnMut(Progress),
    ) -> io::Result<Finished> {
        self.follow(stopper, args, |line| {
            if let Ok(Message::Status(progress)) = serde_json::from_str(line) {
                on_progress(progress);
            }
        })
    }

    pub fn snapshots(&self, stopper: &Stopper, host: &str) -> io::Result<Vec<Snapshot>> {
        let finished = self.run(
            stopper,
            &["snapshots", "--json", "--no-lock", "--host", host],
        )?;
        if finished.exit != Exit::Done {
            return Err(io::Error::other(finished.output));
        }
        let first_json = finished.output.lines().next().unwrap_or("[]");
        let mut snapshots: Vec<Snapshot> =
            serde_json::from_str(first_json).map_err(io::Error::other)?;
        snapshots.sort_by_key(Snapshot::seconds);
        Ok(snapshots)
    }

    pub fn opens(&self, stopper: &Stopper) -> io::Result<Exit> {
        Ok(self.run(stopper, &["cat", "config", "--no-lock"])?.exit)
    }
}

pub fn damaged_packs(check_output: &str) -> Vec<String> {
    check_output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("restic repair packs "))
        .flat_map(str::split_whitespace)
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::damaged_packs;

    #[test]
    fn finds_only_the_packs_restic_says_to_repair() {
        let pack = "035ae823ab30a301cef191b895b5e19a0b38a700ad8c215878ec3aee1ab33aa9";
        let blob = "6bcd619fa7790e0acc7486c8d599d0f0b02e8847b34fdeba7c01591a6e06e618";
        let output = format!(
            "pack {pack} contains 2 errors: [blob {blob}: decrypting failed]\n\nrestic repair packs {pack}\nrestic repair snapshots --forget\n"
        );
        assert_eq!(damaged_packs(&output), vec![pack.to_owned()]);
    }
}
