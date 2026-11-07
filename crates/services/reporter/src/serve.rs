use std::error::Error;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::net::sockopt::socket_peercred;
use serde_json::{Map, Value, json};

use crate::config::{self, Config};
use crate::counters;
use crate::fault::{Fault, capture_hang};
use crate::spool::{MANIFEST, STATE, Spool, Status};

const FIRST_LISTEN_FD: i32 = 3;
const IDLE_EXIT: Duration = Duration::from_secs(30);
const REQUEST_LIMIT: u64 = 16 * 1024;
const REQUEST_PATIENCE: Duration = Duration::from_secs(5);
const MOST_CAPTURES: usize = 2;
const ADMIN_GROUP: &str = "sudo";
const SESSION_KINDS: [&str; 7] = [
    "app-hang",
    "force-quit",
    "compositor-stall",
    "shell-stall",
    "gpu-reset",
    "display-failure",
    "wake-failure",
];
const CAPTURE_KINDS: [&str; 4] = ["app-hang", "force-quit", "compositor-stall", "shell-stall"];

fn is_administrator(uid: u32) -> bool {
    if uid == 0 {
        return true;
    }
    let passwd = fs::read_to_string("/etc/passwd").unwrap_or_default();
    let account = passwd.lines().find_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        (fields.get(2)?.parse() == Ok(uid)).then(|| (*fields.first()?).to_owned())
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

fn program_of(pid: &str) -> Option<String> {
    let path = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    path.file_name()?.to_str().map(str::to_owned)
}

struct Service {
    spool_root: PathBuf,
    config_path: PathBuf,
    captures: AtomicUsize,
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
            fault.program = program_of(pid);
            fault.pid = Some(pid.clone());
        }
        if let Some(Value::Object(detail)) = request.get("detail") {
            fault.detail.extend(detail.clone());
        }
        if let Some(pid) = pid.filter(|_| CAPTURE_KINDS.contains(&kind)) {
            if self.captures.fetch_add(1, Ordering::SeqCst) >= MOST_CAPTURES {
                self.captures.fetch_sub(1, Ordering::SeqCst);
                return Err("too many captures at once".into());
            }
            capture_hang(
                &mut fault,
                &pid,
                request.get("seconds").and_then(Value::as_u64),
            );
            self.captures.fetch_sub(1, Ordering::SeqCst);
        } else {
            fault.signature_text = field(request, "reason").map(str::to_owned);
        }
        let spool = Spool::open(&self.spool_root)?;
        let id = fault.store(&spool, &self.config(), &self.spool_root)?;
        Ok(json!({"id": id}))
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
        json!({
            "sending": config.sends_automatically(),
            "configured": config.server().is_some(),
        })
    }

    fn handle(&self, request: &Map<String, Value>, peer: u32) -> Result<Value, Box<dyn Error>> {
        let id = field(request, "id").unwrap_or_default();
        match field(request, "op").unwrap_or_default() {
            "fault" => self.fault(request, peer),
            "count" => {
                counters::count(&self.spool_root, field(request, "name").unwrap_or_default());
                Ok(json!({}))
            }
            "list" => self.list(peer),
            "read" => self.read(id, peer),
            "status" => Ok(self.status()),
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
                if !on {
                    Spool::open(&self.spool_root)?.clear_outbox()?;
                }
                Ok(self.status())
            }
            other => Err(format!("unknown request: {other}").into()),
        }
    }

    fn connection(&self, stream: UnixStream) -> std::io::Result<()> {
        let peer = socket_peercred(&stream)?.uid.as_raw();
        stream.set_read_timeout(Some(REQUEST_PATIENCE))?;
        stream.set_write_timeout(Some(REQUEST_PATIENCE))?;
        let mut line = String::new();
        BufReader::new(Read::take(&stream, REQUEST_LIMIT)).read_line(&mut line)?;
        let answer = serde_json::from_str(&line)
            .map_err(|error| error.to_string())
            .and_then(|request: Map<String, Value>| {
                self.handle(&request, peer)
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
    });
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
