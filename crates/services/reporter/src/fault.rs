use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Receiver;
use std::thread::sleep;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use ring::digest::{SHA256, digest};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::process::{Pid, PidfdFlags, PidfdGetfdFlags, pidfd_getfd, pidfd_open};
use serde_json::{Map, Value, json};

use crate::config::Config;
use crate::facts;
use crate::hang::{self, HangEnd, Sampler};
use crate::scrub::Scrubber;
use crate::signature::{Frame, signature};
use crate::spool::{Attachment, Spool};
use crate::stacks::{Stacks, parse_eu_stack};

const PARTIAL: &str = "partial";
const FRAME_FORMAT: u64 = 2;
const DEVCOREDUMPS: &str = "/sys/class/devcoredump";
const DEVICE_DUMP_LIMIT: u64 = 8 * 1024 * 1024;
const IDLE_WAIT: &str = "do_epoll_wait";
const SAMPLE_GAP: Duration = Duration::from_millis(500);
const SAMPLE_PATIENCE: Duration = Duration::from_secs(2);
const EU_STACK_CHECK: Duration = Duration::from_millis(20);
const UNSAMPLED_PEERS: [&str; 1] = ["bande-compositor"];
const NETWORK_FILESYSTEMS: [(&str, &str); 7] = [
    ("cifs", "smb"),
    ("smb3", "smb"),
    ("nfs4", "nfs"),
    ("nfs", "nfs"),
    ("rpc_", "nfs"),
    ("fuse", "fuse"),
    ("sshfs", "fuse"),
];

pub struct Fault {
    pub kind: String,
    pub program: Option<String>,
    pub executable: Option<PathBuf>,
    pub owner: Option<u32>,
    pub pid: Option<String>,
    pub detail: Map<String, Value>,
    pub panic_location: Option<String>,
    pub signature_text: Option<String>,
    pub stacks: Stacks,
    pub time: Timestamp,
    pub boot: Option<String>,
    pub attachments: Vec<Attachment>,
    pub description: Option<String>,
}

impl Fault {
    pub fn new(kind: &str) -> Self {
        Self {
            kind: kind.to_owned(),
            program: None,
            executable: None,
            owner: None,
            pid: None,
            detail: Map::new(),
            panic_location: None,
            signature_text: None,
            stacks: Stacks::default(),
            time: Timestamp::now(),
            boot: None,
            attachments: Vec::new(),
            description: None,
        }
    }

    fn signature(&self) -> String {
        let extra = self.signature_text.as_deref().map(|text| Frame {
            symbol: Some(text),
            module: "",
            offset: 0,
        });
        let mut frames: Vec<Frame<'_>> = extra.into_iter().collect();
        frames.extend(self.stacks.faulting());
        signature(
            &format!("{} {}", self.kind, self.program.as_deref().unwrap_or("-")),
            self.panic_location.as_deref(),
            &frames,
        )
    }

    pub fn store(
        mut self,
        spool: &Spool,
        config: &Config,
        spool_root: &Path,
    ) -> io::Result<String> {
        let scrubber = Scrubber::for_this_computer();
        self.signature_text = self
            .signature_text
            .as_deref()
            .map(|text| scrubber.text(text));
        let mut manifest = Map::new();
        let signature = match &config.signature_tag {
            Some(tag) => format!("{tag}-{}", self.signature()),
            None => self.signature(),
        };
        manifest.insert("signature".to_owned(), json!(signature));
        manifest.insert("kind".to_owned(), json!(self.kind));
        manifest.insert("time".to_owned(), json!(self.time.to_string()));
        manifest.insert("program".to_owned(), json!(self.program));
        manifest.insert("owner_uid".to_owned(), json!(self.owner));
        manifest.insert("build_ids".to_owned(), self.stacks.build_ids());
        manifest.insert("threads".to_owned(), self.stacks.threads_json());
        manifest.insert("frame_format".to_owned(), json!(FRAME_FORMAT));
        manifest.insert("images".to_owned(), self.stacks.images_json());
        manifest.insert("panic_location".to_owned(), json!(self.panic_location));
        manifest.insert("signature_text".to_owned(), json!(self.signature_text));
        manifest.insert("counters".to_owned(), facts::counters_of(spool_root));
        let this_boot = facts::boot_id();
        match self.boot.as_deref().map(facts::boot_id_of) {
            Some(about) => {
                if !about.is_empty() {
                    manifest.insert("boot_id".to_owned(), json!(about));
                }
                manifest.insert("reported_from_boot_id".to_owned(), json!(this_boot));
            }
            None => {
                manifest.insert("boot_id".to_owned(), json!(this_boot));
            }
        }
        manifest.extend(facts::machine());
        if let Some(version) = self
            .executable
            .as_deref()
            .and_then(facts::package_version_of)
        {
            manifest.insert("desktop_version".to_owned(), json!(version));
        }
        if let Some(uid) = self.owner {
            facts::session(uid, &mut manifest);
        }
        for (name, value) in std::mem::take(&mut self.detail) {
            let value = match value {
                Value::String(text) => Value::String(scrubber.text(&text)),
                other => other,
            };
            manifest.insert(name, value);
        }
        if let Some(description) = self.description.take() {
            manifest.insert("description".to_owned(), json!(description));
        }
        let journal =
            facts::journal_around(self.boot.as_deref(), Some(self.time.as_second()), &scrubber);
        push_text(&mut self.attachments, "journal.txt", journal);
        if let (Some(uid), Some(program)) = (self.owner, self.program.as_deref()) {
            let ring = facts::ring_lines(uid, program, self.pid.as_deref());
            push_text(&mut self.attachments, "ring.txt", scrubber.text(&ring));
        }
        spool.store(manifest, self.attachments, config.sends_automatically())
    }
}

pub fn push_text(attachments: &mut Vec<Attachment>, name: &str, text: String) {
    if !text.trim().is_empty() {
        attachments.push(Attachment {
            name: name.to_owned(),
            bytes: text.into_bytes(),
        });
    }
}

fn thread_ids(pid: &str) -> Vec<String> {
    let mut ids: Vec<String> = fs::read_dir(format!("/proc/{pid}/task"))
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

fn kernel_state(pid: &str, tid: &str) -> String {
    let read = |name: &str| {
        fs::read_to_string(format!("/proc/{pid}/task/{tid}/{name}"))
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    let stat = read("stat");
    let state = stat
        .rsplit_once(") ")
        .and_then(|(_, rest)| rest.split(' ').next())
        .unwrap_or("?");
    format!(
        "thread {tid} {} state={state} wchan={} syscall={}\n{}\n",
        read("comm"),
        read("wchan"),
        read("syscall").split(' ').next().unwrap_or("?"),
        read("stack"),
    )
}

fn kernel_states(pid: &str) -> String {
    thread_ids(pid)
        .iter()
        .map(|tid| kernel_state(pid, tid))
        .collect()
}

pub(crate) fn user_stacks(pid: &str) -> String {
    let Ok(mut child) = Command::new("eu-stack")
        .args(["-p", pid, "-m", "-b", "-n", "48"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
    else {
        return String::new();
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < SAMPLE_PATIENCE => sleep(EU_STACK_CHECK),
            _ => {
                if let Err(error) = child.kill() {
                    tracing::debug!("cannot stop eu-stack: {error}");
                }
                break;
            }
        }
    }
    child
        .wait_with_output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}

fn network_filesystem_in(stacks: &str) -> Option<&'static str> {
    stacks.lines().find_map(|line| {
        NETWORK_FILESYSTEMS
            .iter()
            .find(|(name, _)| line.contains(name))
            .map(|(_, protocol)| *protocol)
    })
}

fn hashed_servers(protocol: &str) -> Value {
    let mounts = fs::read_to_string("/proc/mounts").unwrap_or_default();
    let hashes: Vec<String> = mounts
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(' ');
            let source = fields.next()?;
            let kind = fields.nth(1)?;
            kind.starts_with(protocol).then(|| {
                let server = source
                    .trim_start_matches('/')
                    .split(['/', ':'])
                    .next()
                    .unwrap_or(source);
                let hash = digest(&SHA256, server.as_bytes());
                hash.as_ref()[..8]
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect()
            })
        })
        .collect();
    json!(hashes)
}

fn first_kernel_frames(snapshot: &str) -> String {
    snapshot
        .lines()
        .filter(|line| line.starts_with('['))
        .take(4)
        .filter_map(|line| line.split_whitespace().nth(1))
        .map(|frame| frame.split('+').next().unwrap_or(frame))
        .collect::<Vec<_>>()
        .join(" ")
}

fn write_partial(spool_root: &Path, name: &str, text: &str) -> io::Result<()> {
    let folder = spool_root.join(PARTIAL);
    fs::create_dir_all(&folder)?;
    let mut file = File::create(folder.join(name))?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    File::open(&folder)?.sync_all()
}

pub fn partial_snapshots(spool_root: &Path) -> Vec<(String, String)> {
    let folder = spool_root.join(PARTIAL);
    let Ok(entries) = fs::read_dir(&folder) else {
        return Vec::new();
    };
    let found: Vec<(String, String)> = entries
        .flatten()
        .filter_map(|entry| {
            Some((
                entry.file_name().into_string().ok()?,
                fs::read_to_string(entry.path()).ok()?,
            ))
        })
        .collect();
    if let Err(error) = fs::remove_dir_all(&folder) {
        tracing::debug!("cannot remove the partial snapshots: {error}");
    }
    found
}

fn waits_with_nothing_ready(pid: &str) -> bool {
    let read = |name: &str| {
        fs::read_to_string(format!("/proc/{pid}/task/{pid}/{name}")).unwrap_or_default()
    };
    if read("wchan").trim() != IDLE_WAIT {
        return false;
    }
    let Some(epoll) = read("syscall")
        .split(' ')
        .nth(1)
        .and_then(|fd| i32::from_str_radix(fd.trim_start_matches("0x"), 16).ok())
    else {
        return false;
    };
    let Some(process) = pid
        .parse()
        .ok()
        .and_then(Pid::from_raw)
        .and_then(|pid| pidfd_open(pid, PidfdFlags::empty()).ok())
    else {
        return false;
    };
    let Ok(epoll) = pidfd_getfd(&process, epoll, PidfdGetfdFlags::empty()) else {
        return false;
    };
    let mut fds = [PollFd::new(&epoll, PollFlags::IN)];
    let now = Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    poll(&mut fds, Some(&now)).is_ok_and(|ready| ready == 0)
}

fn main_thread_waits_idle(pid: &str) -> bool {
    fs::read_to_string(format!("/proc/{pid}/task/{pid}/wchan"))
        .is_ok_and(|wchan| wchan.trim() == IDLE_WAIT)
}

pub struct Capture {
    pub idle: bool,
    pub end: Option<HangEnd>,
}

pub fn capture_hang(
    fault: &mut Fault,
    spool_root: &Path,
    pid: &str,
    seconds: Option<u64>,
    sample_for: Duration,
    ends: Option<&Receiver<HangEnd>>,
) -> Capture {
    let idle_at_first = waits_with_nothing_ready(pid);
    let first = kernel_states(pid);
    let partial = format!("{}-{pid}", fault.kind);
    let header = format!(
        "kind={} program={} pid={pid} stalled_seconds={seconds:?}\n{first}",
        fault.kind,
        fault.program.as_deref().unwrap_or("-"),
    );
    if let Err(error) = write_partial(spool_root, &partial, &header) {
        tracing::debug!("cannot save the first snapshot: {error}");
    }
    let main_tid: u32 = pid.parse().unwrap_or(0);
    let blocker = hang::blocker_of(main_tid);
    let sampled_blocker = blocker
        .as_ref()
        .filter(|blocker| !UNSAMPLED_PEERS.contains(&blocker.program.as_str()));
    let mut own = Sampler::new(main_tid);
    let mut theirs = sampled_blocker.map(|blocker| Sampler::new(blocker.pid));
    let mut main_idle = true;
    let mut end = None;
    let started = Instant::now();
    loop {
        main_idle &= main_thread_waits_idle(pid);
        own.add(parse_eu_stack(&user_stacks(pid), u64::from(main_tid)));
        if let (Some(blocker), Some(sampler)) = (sampled_blocker, theirs.as_mut()) {
            let peer = blocker.pid.to_string();
            sampler.add(parse_eu_stack(&user_stacks(&peer), u64::from(blocker.pid)));
        }
        if started.elapsed() + SAMPLE_GAP > sample_for
            || !Path::new(&format!("/proc/{pid}")).exists()
        {
            break;
        }
        match ends.map(|ends| ends.recv_timeout(SAMPLE_GAP)) {
            Some(Ok(ended)) => {
                end = Some(ended);
                break;
            }
            Some(Err(_)) => {}
            None => sleep(SAMPLE_GAP),
        }
    }
    let snapshots = format!("--- first\n{first}--- last\n{}", kernel_states(pid));
    let idle = idle_at_first && main_idle && waits_with_nothing_ready(pid);
    fault.stacks = own.heaviest();
    let interval = SAMPLE_GAP.as_millis();
    let mut tree = own.tree("hang-tree", None, interval);
    if let Some(blocker) = &blocker {
        let stacks = theirs.as_mut().map(Sampler::heaviest).unwrap_or_default();
        fault.detail.insert(
            "blocked_on".to_owned(),
            hang::blocked_on_json(blocker, &stacks),
        );
        if let Some(sampler) = theirs.as_mut().filter(|sampler| sampler.samples() > 0) {
            tree.push_str(&sampler.tree("blocked-on", Some(blocker.via), interval));
        }
    }
    if own.samples() > 0 {
        push_text(&mut fault.attachments, "hang-tree.txt", tree);
    }
    if let Some(protocol) = network_filesystem_in(&snapshots) {
        fault.kind = "mount-stall".to_owned();
        fault.detail.insert("protocol".to_owned(), json!(protocol));
        fault
            .detail
            .insert("servers_hashed".to_owned(), hashed_servers(protocol));
    }
    if let Some(seconds) = seconds {
        fault
            .detail
            .insert("stalled_seconds".to_owned(), json!(seconds));
    }
    if fault.stacks.threads.is_empty() {
        fault.signature_text = Some(first_kernel_frames(&snapshots));
    }
    fault.detail.insert(
        "stuck_main_thread".to_owned(),
        json!(snapshots.lines().nth(1)),
    );
    push_text(&mut fault.attachments, "kernel-stacks.txt", snapshots);
    let done = spool_root.join(PARTIAL).join(&partial);
    if let Err(error) = fs::remove_file(done) {
        tracing::debug!("cannot remove a finished snapshot: {error}");
    }
    Capture { idle, end }
}

pub fn attach_device_dumps(fault: &mut Fault) {
    let Ok(dumps) = fs::read_dir(DEVCOREDUMPS) else {
        return;
    };
    for dump in dumps.flatten() {
        let path = dump.path();
        let driver = fs::read_link(path.join("failing_device/driver"))
            .ok()
            .and_then(|driver| driver.file_name()?.to_str().map(str::to_owned))
            .unwrap_or_else(|| "device".to_owned());
        let mut bytes = Vec::new();
        let read = File::open(path.join("data"))
            .and_then(|data| data.take(DEVICE_DUMP_LIMIT).read_to_end(&mut bytes));
        if let Err(error) = read {
            tracing::debug!("cannot read a device core dump: {error}");
            continue;
        }
        fault.attachments.push(Attachment {
            name: format!("devcoredump-{driver}.bin"),
            bytes,
        });
        if let Err(error) = fs::write(path.join("data"), b"1") {
            tracing::debug!("cannot release a device core dump: {error}");
        }
    }
}

pub fn stuck_tasks() -> String {
    let mut text = String::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return text;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().into_string().ok() else {
            continue;
        };
        if !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        for tid in thread_ids(&pid) {
            let stat =
                fs::read_to_string(format!("/proc/{pid}/task/{tid}/stat")).unwrap_or_default();
            if stat
                .rsplit_once(") ")
                .is_some_and(|(_, rest)| rest.starts_with('D'))
            {
                text.push_str(&kernel_state(&pid, &tid));
            }
        }
    }
    text
}

pub fn mount_stall_details(stuck: &str, fault: &mut Fault) {
    if let Some(protocol) = network_filesystem_in(stuck) {
        fault.kind = "mount-stall".to_owned();
        fault.detail.insert("protocol".to_owned(), json!(protocol));
        fault
            .detail
            .insert("servers_hashed".to_owned(), hashed_servers(protocol));
    }
}
