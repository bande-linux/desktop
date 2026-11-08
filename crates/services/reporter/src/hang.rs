use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use rustix::net::sockopt::socket_peercred;
use rustix::process::{Pid, PidfdFlags, PidfdGetfdFlags, pidfd_getfd, pidfd_open};
use serde_json::{Value, json};

use crate::stacks::{ParsedFrame, Stacks};

#[cfg(target_arch = "aarch64")]
const READ_SYSCALLS: &[i64] = &[63, 65, 67, 69, 207, 212];
#[cfg(target_arch = "aarch64")]
const POLL_SYSCALLS: &[i64] = &[73];
#[cfg(target_arch = "x86_64")]
const READ_SYSCALLS: &[i64] = &[0, 17, 19, 45, 47, 295];
#[cfg(target_arch = "x86_64")]
const POLL_SYSCALLS: &[i64] = &[7, 271];
#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
const READ_SYSCALLS: &[i64] = &[];
#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
const POLL_SYSCALLS: &[i64] = &[];
const WRITE_ACCESS: u32 = 0o1;
const ACCESS_MODE: u32 = 0o3;
const BLOCKED_FRAMES: usize = 16;
const INDENT: usize = 2;

pub struct HangEnd {
    pub seconds: f64,
    pub recovered: bool,
}

struct Waiter {
    owner: u32,
    sender: Sender<HangEnd>,
    receiver: Option<Receiver<HangEnd>>,
}

fn waiter(owner: u32) -> Waiter {
    let (sender, receiver) = channel();
    Waiter {
        owner,
        sender,
        receiver: Some(receiver),
    }
}

#[derive(Default)]
pub struct HangEnds(Mutex<HashMap<(u32, u64), Waiter>>);

impl HangEnds {
    pub fn wait_for(&self, pid: u32, hang: u64, owner: u32) -> Option<Receiver<HangEnd>> {
        let mut waiters = self.0.lock().ok()?;
        waiters
            .entry((pid, hang))
            .or_insert_with(|| waiter(owner))
            .receiver
            .take()
    }

    pub fn forget(&self, pid: u32, hang: u64) {
        if let Ok(mut waiters) = self.0.lock() {
            waiters.remove(&(pid, hang));
        }
    }

    pub fn end(
        &self,
        pid: u32,
        hang: u64,
        peer: u32,
        owns_process: bool,
        end: HangEnd,
    ) -> Result<(), &'static str> {
        let mut waiters = self.0.lock().map_err(|_| "the hang list is broken")?;
        let waiter = match waiters.entry((pid, hang)) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) if owns_process => entry.insert(waiter(peer)),
            Entry::Vacant(_) => return Err("no such hang"),
        };
        if peer != 0 && waiter.owner != peer {
            return Err("that hang belongs to someone else");
        }
        waiter
            .sender
            .send(end)
            .map_err(|_| "that hang report is finished")
    }
}

pub struct Blocker {
    pub pid: u32,
    pub program: String,
    pub via: &'static str,
}

fn number(word: &str) -> Option<u64> {
    u64::from_str_radix(word.trim_start_matches("0x"), 16).ok()
}

fn polled_fd(pid: u32, pollfds: u64) -> Option<i32> {
    let memory = File::open(format!("/proc/{pid}/mem")).ok()?;
    let mut fd = [0u8; 4];
    memory.read_exact_at(&mut fd, pollfds).ok()?;
    Some(i32::from_ne_bytes(fd))
}

fn waited_fd(pid: u32) -> Option<i32> {
    let syscall = fs::read_to_string(format!("/proc/{pid}/task/{pid}/syscall")).ok()?;
    let mut words = syscall.split_whitespace();
    let call: i64 = words.next()?.parse().ok()?;
    let arguments: Vec<u64> = words.take(2).filter_map(number).collect();
    if READ_SYSCALLS.contains(&call) {
        i32::try_from(*arguments.first()?).ok()
    } else if POLL_SYSCALLS.contains(&call) && arguments.get(1) == Some(&1) {
        polled_fd(pid, *arguments.first()?)
    } else {
        None
    }
}

fn socket_peer(pid: u32, fd: i32) -> Option<u32> {
    let process = pidfd_open(Pid::from_raw(pid.cast_signed())?, PidfdFlags::empty()).ok()?;
    let socket = pidfd_getfd(&process, fd, PidfdGetfdFlags::empty()).ok()?;
    let credentials = socket_peercred(&socket).ok()?;
    Some(credentials.pid.as_raw_nonzero().get().cast_unsigned())
}

fn writes_to(pid: &str, fd: &str) -> bool {
    fs::read_to_string(format!("/proc/{pid}/fdinfo/{fd}"))
        .ok()
        .and_then(|info| {
            info.lines()
                .find_map(|line| line.strip_prefix("flags:"))
                .and_then(|flags| u32::from_str_radix(flags.trim(), 8).ok())
        })
        .is_some_and(|flags| flags & ACCESS_MODE == WRITE_ACCESS)
}

fn pipe_writer(reader: u32, pipe: &Path) -> Option<u32> {
    let reader = reader.to_string();
    fs::read_dir("/proc").ok()?.flatten().find_map(|process| {
        let pid = process.file_name().into_string().ok()?;
        if pid == reader || !pid.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let writes = fs::read_dir(process.path().join("fd"))
            .ok()?
            .flatten()
            .any(|fd| {
                fs::read_link(fd.path()).is_ok_and(|target| target == pipe)
                    && fd
                        .file_name()
                        .to_str()
                        .is_some_and(|fd| writes_to(&pid, fd))
            });
        writes.then(|| pid.parse().ok())?
    })
}

pub fn program_of(pid: u32) -> String {
    fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .and_then(|path| path.file_name()?.to_str().map(str::to_owned))
        .or_else(|| {
            fs::read_to_string(format!("/proc/{pid}/comm"))
                .ok()
                .map(|comm| comm.trim().to_owned())
        })
        .unwrap_or_else(|| "?".to_owned())
}

pub fn blocker_of(pid: u32) -> Option<Blocker> {
    let fd = waited_fd(pid)?;
    let target = fs::read_link(format!("/proc/{pid}/fd/{fd}")).ok()?;
    let name = target.to_str()?;
    let (via, peer) = if name.starts_with("socket:") {
        ("socket", socket_peer(pid, fd)?)
    } else if name.starts_with("pipe:") {
        ("pipe", pipe_writer(pid, &target)?)
    } else {
        return None;
    };
    (peer != pid).then(|| Blocker {
        pid: peer,
        program: program_of(peer),
        via,
    })
}

#[derive(Default)]
struct Node {
    label: String,
    count: u32,
    children: Vec<Node>,
}

impl Node {
    fn add(&mut self, labels: &[String]) {
        self.count += 1;
        let Some((first, rest)) = labels.split_first() else {
            return;
        };
        let index = self
            .children
            .iter()
            .position(|child| &child.label == first)
            .unwrap_or_else(|| {
                self.children.push(Node {
                    label: first.clone(),
                    ..Node::default()
                });
                self.children.len() - 1
            });
        self.children[index].add(rest);
    }

    fn render(&mut self, depth: usize, out: &mut String) {
        self.children
            .sort_by_key(|child| std::cmp::Reverse(child.count));
        for child in &mut self.children {
            out.push_str(&format!(
                "{:indent$}{} {}\n",
                "",
                child.count,
                child.label,
                indent = depth * INDENT
            ));
            child.render(depth + 1, out);
        }
    }
}

fn label(frame: &ParsedFrame) -> String {
    format!(
        "{} {}+{:#x}",
        frame.symbol.as_deref().unwrap_or("?"),
        frame.module,
        frame.offset
    )
}

pub struct Sampler {
    pid: u32,
    samples: u32,
    threads: BTreeMap<u64, Node>,
    main_stacks: Vec<(Vec<String>, u32, Stacks)>,
}

impl Sampler {
    pub fn new(pid: u32) -> Self {
        Self {
            pid,
            samples: 0,
            threads: BTreeMap::new(),
            main_stacks: Vec::new(),
        }
    }

    pub fn add(&mut self, stacks: Stacks) {
        if stacks.threads.is_empty() {
            return;
        }
        self.samples += 1;
        let mut main = Vec::new();
        for thread in &stacks.threads {
            let labels: Vec<String> = thread.frames.iter().rev().map(label).collect();
            self.threads.entry(thread.id).or_default().add(&labels);
            if thread.crashed {
                main = labels;
            }
        }
        match self.main_stacks.iter_mut().find(|(seen, ..)| *seen == main) {
            Some((_, count, _)) => *count += 1,
            None => self.main_stacks.push((main, 1, stacks)),
        }
    }

    pub fn samples(&self) -> u32 {
        self.samples
    }

    pub fn heaviest(&mut self) -> Stacks {
        let heaviest = self
            .main_stacks
            .iter()
            .enumerate()
            .max_by_key(|(index, (_, count, _))| (*count, std::cmp::Reverse(*index)))
            .map(|(index, _)| index);
        heaviest
            .map(|index| self.main_stacks.swap_remove(index).2)
            .unwrap_or_default()
    }

    pub fn tree(&mut self, header: &str, via: Option<&str>, interval_ms: u128) -> String {
        let pid = self.pid;
        let via = via.map(|via| format!(" via {via}")).unwrap_or_default();
        let mut out = format!(
            "{header} {} pid {pid}{via} samples {} interval-ms {interval_ms}\n",
            program_of(pid),
            self.samples
        );
        for (tid, node) in &mut self.threads {
            let name = fs::read_to_string(format!("/proc/{pid}/task/{tid}/comm"))
                .map_or_else(|_| "?".to_owned(), |name| name.trim().to_owned());
            out.push_str(&format!("thread {tid} {name}\n"));
            node.render(0, &mut out);
        }
        out
    }
}

pub fn blocked_on_json(blocker: &Blocker, stacks: &Stacks) -> Value {
    let mut frames = stacks.faulting_frames_json();
    frames.truncate(BLOCKED_FRAMES);
    json!({"program": blocker.program, "via": blocker.via, "frames": frames})
}
