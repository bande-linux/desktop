use std::error::Error;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::net::sockopt::socket_peercred;
use serde_json::{Map, Value, json};

use crate::checkin;
use crate::config::{self, Config};
use crate::counters;
use crate::diagnose::diagnose;
use crate::facts;
use crate::fault::{Fault, capture_hang};
use crate::hang::{HangEnd, HangEnds};
use crate::spool::{MANIFEST, STATE, Spool, Status};

const FAILED: &str = "failed";
const LONGEST_UNIT: usize = 200;
const NOT_INSTALLED: &str = "only installed programs leave fault reports";
const FROM_INSTALLED_PROGRAMS: [&str; 3] = ["fault", "count", "hang-over"];
const FIRST_LISTEN_FD: i32 = 3;
const IDLE_EXIT: Duration = Duration::from_secs(30);
const REQUEST_LIMIT: u64 = 16 * 1024;
const REQUEST_PATIENCE: Duration = Duration::from_secs(5);
const MOST_CAPTURES: usize = 2;
const ADMIN_GROUP: &str = "sudo";
const SESSION_KINDS: [&str; 8] = [
    "oom",
    "app-hang",
    "force-quit",
    "compositor-stall",
    "shell-stall",
    "gpu-reset",
    "display-failure",
    "wake-failure",
];
const CAPTURE_KINDS: [&str; 4] = ["app-hang", "force-quit", "compositor-stall", "shell-stall"];
const GUARDED_KINDS: [&str; 3] = ["app-hang", "compositor-stall", "shell-stall"];
const IDLE_HANG_DROPPED: &str = "idle-hang-dropped";
const PINGED_KINDS: [&str; 2] = ["app-hang", "shell-stall"];
const PINGED_SAMPLING: Duration = Duration::from_secs(10);
const OTHER_SAMPLING: Duration = Duration::from_secs(3);
const TERMINATIONS: [&str; 2] = ["logout-timeout", "force-quit"];
const HANG_END_PATIENCE: Duration = Duration::from_secs(120);

fn is_administrator(uid: u32) -> bool {
    if uid == 0 {
        return true;
    }
    let passwd = fs::read_to_string("/etc/passwd").unwrap_or_default();
    let account = passwd.lines().find_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        (fields.get(2)?.parse() == Ok(uid))
            .then(|| fields.first().map(|name| (*name).to_owned()))
            .flatten()
    });
    let Some(account) = account else {
        return false;
    };
    fs::read_to_string("/etc/group")
        .unwrap_or_default()
        .lines()
        .any(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            fields.first() == Some(&ADMIN_GROUP)
                && fields
                    .get(3)
                    .is_some_and(|members| members.split(',').any(|member| member == account))
        })
}

fn process_owner(pid: &str) -> Option<u32> {
    fs::metadata(format!("/proc/{pid}")).ok().map(|m| m.uid())
}

fn executable(pid: &str) -> Option<PathBuf> {
    fs::read_link(format!("/proc/{pid}/exe")).ok()
}

fn runs_elsewhere(executable: Option<&Path>) -> bool {
    executable.is_some_and(|path| !bande_log::is_installed_program(path))
}

fn collector(failed_unit: Option<&str>) -> String {
    let instance = failed_unit
        .filter(|unit| {
            !unit.is_empty()
                && unit.len() <= LONGEST_UNIT
                && unit
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.@:".contains(&byte))
        })
        .map_or_else(|| FAILED.to_owned(), |unit| format!("user-{unit}"));
    format!("bande-crash-reports@{instance}.service")
}

fn start_collector(failed_unit: Option<&str>) -> std::io::Result<()> {
    let status = Command::new("systemctl")
        .args(["start", "--no-block", &collector(failed_unit)])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other("cannot start the collector"));
    }
    Ok(())
}

struct Peer {
    uid: u32,
    elsewhere: bool,
}

struct Service {
    spool_root: PathBuf,
    config_path: PathBuf,
    captures: AtomicUsize,
    hang_ends: HangEnds,
}

struct Waiting<'a> {
    ends: &'a HangEnds,
    pid: u32,
    hang: u64,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.ends.forget(self.pid, self.hang);
    }
}

fn field<'a>(request: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    request.get(name).and_then(Value::as_str)
}

fn may_see(report_owner: Option<u64>, peer: u32, administrator: bool) -> bool {
    administrator || report_owner == Some(u64::from(peer))
}

impl Service {
    fn config(&self) -> Config {
        Config::load(&self.config_path).unwrap_or_default()
    }

    fn fault(&self, request: &Map<String, Value>, peer: u32) -> Result<Value, Box<dyn Error>> {
        let kind = field(request, "kind").unwrap_or_default();
        if !SESSION_KINDS.contains(&kind) {
            return Err(format!("not a fault kind: {kind}").into());
        }
        let mut fault = Fault::new(kind);
        fault.owner = Some(peer);
        let pid = request
            .get("pid")
            .and_then(Value::as_u64)
            .map(|pid| pid.to_string());
        if let Some(pid) = &pid {
            if peer != 0 && process_owner(pid) != Some(peer) {
                return Err("that process belongs to someone else".into());
            }
            let executable = executable(pid);
            if runs_elsewhere(executable.as_deref()) {
                return Err(NOT_INSTALLED.into());
            }
            fault.program = executable
                .as_deref()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .map(str::to_owned);
            fault.executable = executable;
            fault.pid = Some(pid.clone());
        }
        if let Some(Value::Object(detail)) = request.get("detail") {
            fault.detail.extend(detail.clone());
        }
        if kind == "force-quit" {
            fault
                .detail
                .entry("termination")
                .or_insert_with(|| json!("force-quit"));
        }
        if kind == "oom" {
            self.oom(&mut fault, request)?;
        }
        if let Some(pid) = pid.filter(|_| CAPTURE_KINDS.contains(&kind)) {
            let started = Instant::now();
            let seconds = request.get("seconds").and_then(Value::as_u64);
            let waiting = request
                .get("hang")
                .and_then(Value::as_u64)
                .zip(pid.parse().ok())
                .filter(|_| PINGED_KINDS.contains(&kind))
                .map(|(hang, pid)| Waiting {
                    ends: &self.hang_ends,
                    pid,
                    hang,
                });
            let ends = waiting
                .as_ref()
                .and_then(|waiting| self.hang_ends.wait_for(waiting.pid, waiting.hang, peer));
            if self.captures.fetch_add(1, Ordering::SeqCst) >= MOST_CAPTURES {
                self.captures.fetch_sub(1, Ordering::SeqCst);
                return Err("too many captures at once".into());
            }
            let sample_for = if PINGED_KINDS.contains(&kind) {
                PINGED_SAMPLING
            } else {
                OTHER_SAMPLING
            };
            let capture = capture_hang(
                &mut fault,
                &self.spool_root,
                &pid,
                seconds,
                sample_for,
                ends.as_ref(),
            );
            self.captures.fetch_sub(1, Ordering::SeqCst);
            if capture.idle && GUARDED_KINDS.contains(&kind) {
                counters::count(&self.spool_root, IDLE_HANG_DROPPED);
                return Err("the process was waiting idle in its event loop".into());
            }
            if let Some(ends) = &ends {
                let end = capture.end.or_else(|| {
                    ends.recv_timeout(HANG_END_PATIENCE.saturating_sub(started.elapsed()))
                        .ok()
                });
                let (hang_seconds, recovered) = match end {
                    Some(end) => {
                        if let Some(termination) = end.termination {
                            fault
                                .detail
                                .insert("termination".to_owned(), json!(termination));
                        }
                        (end.seconds, end.recovered)
                    }
                    None => (
                        seconds.unwrap_or(0) as f64 + started.elapsed().as_secs_f64(),
                        false,
                    ),
                };
                fault.detail.insert(
                    "hang_seconds".to_owned(),
                    json!(hang_seconds.round() as u64),
                );
                fault
                    .detail
                    .insert("recovered".to_owned(), json!(recovered));
            } else if PINGED_KINDS.contains(&kind) {
                fault
                    .detail
                    .insert("hang_seconds".to_owned(), json!(seconds));
                fault.detail.insert("recovered".to_owned(), json!(false));
            }
            drop(waiting);
        } else {
            fault.signature_text = field(request, "reason").map(str::to_owned);
        }
        let spool = Spool::open(&self.spool_root)?;
        let id = fault.store(&spool, &self.config(), &self.spool_root)?;
        Ok(json!({"id": id}))
    }

    fn oom(&self, fault: &mut Fault, request: &Map<String, Value>) -> Result<(), Box<dyn Error>> {
        let unit = field(request, "unit").unwrap_or_default();
        let reason = facts::oomd_reason(unit, fault.time.as_second())
            .ok_or("no systemd-oomd kill of that unit; the kernel watcher reports kernel kills")?;
        fault.program = field(request, "app")
            .filter(|app| !app.is_empty())
            .unwrap_or(unit)
            .to_owned()
            .into();
        fault.detail.insert("unit".to_owned(), json!(unit));
        fault.detail.insert("termination".to_owned(), json!("oomd"));
        fault.detail.insert("limit".to_owned(), json!(reason));
        facts::add_memory_details(&mut fault.detail);
        Ok(())
    }

    fn hang_over(&self, request: &Map<String, Value>, peer: u32) -> Result<Value, Box<dyn Error>> {
        let number = |name| request.get(name).and_then(Value::as_u64);
        let (Some(pid), Some(hang)) = (number("pid"), number("hang")) else {
            return Err("a hang end needs pid and hang".into());
        };
        let pid = u32::try_from(pid)?;
        let owns_process =
            process_owner(&pid.to_string()).is_some_and(|owner| peer == 0 || owner == peer);
        let end = HangEnd {
            seconds: request
                .get("seconds")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            recovered: request
                .get("recovered")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            termination: field(request, "termination")
                .filter(|termination| TERMINATIONS.contains(termination))
                .map(str::to_owned),
        };
        self.hang_ends.end(pid, hang, peer, owns_process, end)?;
        Ok(json!({}))
    }

    fn list(&self, peer: u32) -> Result<Value, Box<dyn Error>> {
        let spool = Spool::open(&self.spool_root)?;
        let waiting = spool.waiting()?;
        let administrator = is_administrator(peer);
        let reports: Vec<Value> = spool
            .all()
            .iter()
            .filter(|report| {
                may_see(
                    report.manifest.get("owner_uid").and_then(Value::as_u64),
                    peer,
                    administrator,
                )
            })
            .map(|report| {
                let status = match report.status {
                    Status::Sent => "sent",
                    Status::Rejected => "rejected",
                    Status::Local if waiting.contains(&report.id) => "waiting",
                    Status::Local => "local",
                };
                json!({
                    "id": report.id,
                    "kind": report.manifest.get("kind"),
                    "program": report.manifest.get("program"),
                    "time": report.manifest.get("time"),
                    "occurrences": report.occurrences,
                    "bytes": report.bytes,
                    "status": status,
                })
            })
            .collect();
        Ok(json!({"reports": reports}))
    }

    fn visible(
        &self,
        id: &str,
        peer: u32,
    ) -> Result<(Spool, crate::spool::Stored), Box<dyn Error>> {
        let spool = Spool::open(&self.spool_root)?;
        let report = spool.read(id)?;
        let owner = report.manifest.get("owner_uid").and_then(Value::as_u64);
        if !may_see(owner, peer, is_administrator(peer)) {
            return Err("that report belongs to someone else".into());
        }
        Ok((spool, report))
    }

    fn read(&self, id: &str, peer: u32) -> Result<Value, Box<dyn Error>> {
        let (_, report) = self.visible(id, peer)?;
        let mut files = Map::new();
        for entry in fs::read_dir(&report.path)?.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name == STATE || name.starts_with('.') {
                continue;
            }
            let bytes = fs::read(entry.path())?;
            let content = match std::str::from_utf8(&bytes) {
                Ok(text) => json!(text),
                Err(_) => json!(format!("(binary, {} bytes)", bytes.len())),
            };
            files.insert(name, content);
        }
        let manifest = files.remove(MANIFEST);
        Ok(json!({"manifest": manifest, "attachments": files}))
    }

    fn status(&self) -> Value {
        let config = self.config();
        let checkin = config
            .sends_automatically()
            .then(|| {
                let spool = Spool::open(&self.spool_root).ok()?;
                let day = checkin::due_days(&self.spool_root)
                    .last()
                    .copied()
                    .or_else(checkin::yesterday)?;
                checkin::payload(&spool, day).ok()
            })
            .flatten();
        json!({
            "sending": config.sends_automatically(),
            "configured": config.server().is_some(),
            "checkin": checkin,
        })
    }

    fn handle(&self, request: &Map<String, Value>, peer: &Peer) -> Result<Value, Box<dyn Error>> {
        let id = field(request, "id").unwrap_or_default();
        let op = field(request, "op").unwrap_or_default();
        if FROM_INSTALLED_PROGRAMS.contains(&op) && peer.elsewhere {
            return Err(NOT_INSTALLED.into());
        }
        let peer = peer.uid;
        match op {
            "fault" => self.fault(request, peer),
            "hang-over" => self.hang_over(request, peer),
            "count" => {
                counters::count(&self.spool_root, field(request, "name").unwrap_or_default());
                Ok(json!({}))
            }
            "list" => self.list(peer),
            "read" => self.read(id, peer),
            "status" => Ok(self.status()),
            "collect" => {
                start_collector(field(request, "unit"))?;
                Ok(json!({}))
            }
            "diagnose" => {
                let description = field(request, "description").unwrap_or_default();
                let send = request
                    .get("send")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let id = diagnose(&self.spool_root, &self.config(), peer, description, send)?;
                Ok(json!({
                    "id": id,
                    "queued": send && self.config().server().is_some(),
                }))
            }
            "delete" => {
                let (spool, _) = self.visible(id, peer)?;
                spool.delete(id)?;
                Ok(json!({}))
            }
            "send" => {
                let (spool, _) = self.visible(id, peer)?;
                if self.config().server().is_none() {
                    return Err("no server is configured".into());
                }
                spool.queue(id)?;
                Ok(json!({}))
            }
            "set-sending" => {
                if !is_administrator(peer) {
                    return Err("only an administrator can change this".into());
                }
                let on = request.get("on").and_then(Value::as_bool).unwrap_or(false);
                config::set_sending(&self.config_path, on)?;
                let spool = Spool::open(&self.spool_root)?;
                if on {
                    checkin::queue_if_due(&spool, &self.config());
                } else {
                    spool.clear_outbox()?;
                    checkin::forget(&self.spool_root)?;
                }
                Ok(self.status())
            }
            other => Err(format!("unknown request: {other}").into()),
        }
    }

    fn connection(&self, stream: UnixStream) -> std::io::Result<()> {
        let credentials = socket_peercred(&stream)?;
        let peer = Peer {
            uid: credentials.uid.as_raw(),
            elsewhere: runs_elsewhere(
                executable(&credentials.pid.as_raw_nonzero().to_string()).as_deref(),
            ),
        };
        stream.set_read_timeout(Some(REQUEST_PATIENCE))?;
        stream.set_write_timeout(Some(REQUEST_PATIENCE))?;
        let mut line = String::new();
        BufReader::new(Read::take(&stream, REQUEST_LIMIT)).read_line(&mut line)?;
        let answer = serde_json::from_str(&line)
            .map_err(|error| error.to_string())
            .and_then(|request: Map<String, Value>| {
                self.handle(&request, &peer)
                    .map_err(|error| error.to_string())
            });
        let answer = match answer {
            Ok(value) => json!({"ok": value}),
            Err(error) => {
                tracing::debug!("refused a request: {error}");
                json!({"error": error})
            }
        };
        let mut stream = stream;
        writeln!(stream, "{answer}")
    }
}

pub fn run(spool_root: &Path, config_path: &Path) -> Result<(), Box<dyn Error>> {
    let listening = std::env::var("LISTEN_FDS").is_ok_and(|fds| fds == "1")
        && std::env::var("LISTEN_PID").is_ok_and(|pid| pid == std::process::id().to_string());
    if !listening {
        return Err("serve runs from its systemd socket".into());
    }
    let listen = unsafe { OwnedFd::from_raw_fd(FIRST_LISTEN_FD) };
    let listener = UnixListener::from(listen);
    let service = Arc::new(Service {
        spool_root: spool_root.to_path_buf(),
        config_path: config_path.to_path_buf(),
        captures: AtomicUsize::new(0),
        hang_ends: HangEnds::default(),
    });
    checkin::queue_if_due(&Spool::open(spool_root)?, &service.config());
    let running = Arc::new(AtomicUsize::new(0));
    loop {
        let mut fds = [PollFd::new(&listener, PollFlags::IN)];
        let timeout = Timespec {
            tv_sec: IDLE_EXIT.as_secs() as i64,
            tv_nsec: 0,
        };
        match poll(&mut fds, Some(&timeout)) {
            Ok(0) if running.load(Ordering::SeqCst) == 0 => return Ok(()),
            Ok(0) | Err(rustix::io::Errno::INTR) => continue,
            Ok(_) => {}
            Err(error) => return Err(error.into()),
        }
        let (stream, _) = listener.accept()?;
        let service = Arc::clone(&service);
        let counted = Arc::clone(&running);
        counted.fetch_add(1, Ordering::SeqCst);
        let spawned = std::thread::Builder::new()
            .name("request".to_owned())
            .spawn(move || {
                if let Err(error) = service.connection(stream) {
                    tracing::debug!("a request failed: {error}");
                }
                counted.fetch_sub(1, Ordering::SeqCst);
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot start a request: {error}");
            running.fetch_sub(1, Ordering::SeqCst);
        }
    }
}
