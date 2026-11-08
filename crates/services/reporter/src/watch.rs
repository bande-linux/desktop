use std::collections::VecDeque;
use std::error::Error;
use std::fs::{self, File};
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::OFlags;
use serde_json::json;

use crate::config::Config;
use crate::counters;
use crate::facts;
use crate::fault::{Fault, attach_device_dumps, mount_stall_details, push_text, stuck_tasks};
use crate::scrub::Scrubber;
use crate::spool::Spool;

const KMSG: &str = "/dev/kmsg";
const SEEN: &str = "/run/bande/kmsg-seq";
const SETTLE: Duration = Duration::from_secs(2);
const KEPT_LINES: usize = 200;
const RECORD_BYTES: usize = 8192;
const NOTICE: u8 = 5;
const OOM_SUMMARY: &str = "oom-kill:";
const PRESSURE: &str = "/proc/pressure/memory";
const PRESSURE_TRIGGER: &[u8] = b"full 1000000 10000000\0";
const PRESSURE_SETTLE: Duration = Duration::from_secs(30);
const PRESSURE_QUIET: Duration = Duration::from_secs(60 * 60);

const FAULTS: [(&str, &str); 26] = [
    ("Unable to handle kernel", "kernel-oops"),
    ("Internal error: Oops", "kernel-oops"),
    ("kernel BUG at", "kernel-oops"),
    ("BUG: ", "kernel-oops"),
    ("WARNING: CPU:", "kernel-warning"),
    ("blocked for more than", "hung-task"),
    ("soft lockup", "lockup"),
    ("hard LOCKUP", "lockup"),
    ("rcu_preempt detected stalls", "lockup"),
    ("rcu_sched detected stalls", "lockup"),
    ("Out of memory: Killed process", "oom"),
    ("Memory cgroup out of memory", "oom"),
    ("GPU HANG", "gpu-error"),
    ("GPU reset", "gpu-error"),
    ("GPU fault", "gpu-error"),
    ("GPU timeout", "gpu-error"),
    ("job timeout", "gpu-error"),
    ("ring gfx timeout", "gpu-error"),
    ("flip_done timed out", "display-error"),
    ("atomic commit failed", "display-error"),
    ("modeset failed", "display-error"),
    ("Some devices failed to suspend", "sleep-failed"),
    ("PM: failed to", "sleep-failed"),
    ("Failed to resume", "sleep-failed"),
    ("EXT4-fs error", "storage-error"),
    ("BTRFS error", "storage-error"),
];
const EXPECTED: [(&str, &str); 5] = [
    ("has not responded in", "network-server-offline"),
    ("not responding, timed out", "network-server-offline"),
    ("not responding, still trying", "network-server-offline"),
    ("deauthenticating from", "wifi-drop"),
    ("disconnected from AP", "wifi-drop"),
];

pub struct Record {
    pub level: u8,
    pub sequence: u64,
    pub text: String,
}

pub fn parse_record(line: &str) -> Option<Record> {
    let (head, text) = line.split_once(';')?;
    let mut fields = head.split(',');
    let priority: u32 = fields.next()?.parse().ok()?;
    let sequence = fields.next()?.parse().ok()?;
    Some(Record {
        level: (priority & 7) as u8,
        sequence,
        text: text.lines().next().unwrap_or_default().to_owned(),
    })
}

pub fn fault_kind(text: &str) -> Option<&'static str> {
    FAULTS
        .iter()
        .find(|(pattern, _)| text.contains(pattern))
        .map(|(_, kind)| *kind)
}

pub fn expected_event(text: &str) -> Option<&'static str> {
    EXPECTED
        .iter()
        .find(|(pattern, _)| text.contains(pattern))
        .map(|(_, name)| *name)
}

fn stable_text(text: &str) -> String {
    text.split_whitespace()
        .filter(|word| !word.bytes().any(|byte| byte.is_ascii_digit()))
        .take(8)
        .collect::<Vec<_>>()
        .join(" ")
}

fn first_call_trace_frame(lines: &VecDeque<String>) -> Option<String> {
    let mut lines = lines
        .iter()
        .skip_while(|line| !line.contains("Call trace:"));
    lines.next()?;
    let frame = lines.next()?.split_whitespace().next()?;
    Some(frame.split('+').next().unwrap_or(frame).to_owned())
}

fn oom_details(lines: &VecDeque<String>, fault: &mut Fault) {
    facts::add_memory_details(&mut fault.detail);
    fault
        .detail
        .insert("termination".to_owned(), json!("kernel-oom"));
    let Some(summary) = lines
        .iter()
        .rev()
        .find_map(|line| line.strip_prefix(OOM_SUMMARY))
    else {
        return;
    };
    let field = |name: &str| {
        summary
            .split(',')
            .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
    };
    let limit = match field("constraint") {
        Some("CONSTRAINT_MEMCG") => format!("cgroup {}", field("oom_memcg").unwrap_or("?")),
        Some("CONSTRAINT_NONE") | None => "system".to_owned(),
        Some(other) => other.trim_start_matches("CONSTRAINT_").to_lowercase(),
    };
    fault.detail.insert("limit".to_owned(), json!(limit));
    if let Some(unit) = field("task_memcg").and_then(|memcg| memcg.rsplit('/').next()) {
        fault.detail.insert("unit".to_owned(), json!(unit));
    }
    if let Some(task) = field("task") {
        fault.program = Some(task.to_owned());
    }
}

fn open_pressure_trigger() -> Option<File> {
    let file = File::options()
        .read(true)
        .write(true)
        .custom_flags((OFlags::NONBLOCK | OFlags::CLOEXEC).bits() as i32)
        .open(PRESSURE)
        .ok()?;
    rustix::io::write(&file, PRESSURE_TRIGGER).ok()?;
    Some(file)
}

fn report_pressure(spool: &Spool, config_path: &Path, spool_root: &Path) {
    let mut fault = Fault::new("memory-pressure");
    fault.program = Some("system".to_owned());
    facts::add_memory_details(&mut fault.detail);
    let config = Config::load(config_path).unwrap_or_default();
    if let Err(error) = fault.store(spool, &config, spool_root) {
        tracing::warn!("cannot store a memory pressure report: {error}");
    }
}

fn left(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

struct Pending {
    kind: &'static str,
    first: String,
    since: Instant,
}

fn report(
    pending: &Pending,
    lines: &VecDeque<String>,
    spool: &Spool,
    config_path: &Path,
    spool_root: &Path,
) {
    let scrubber = Scrubber::for_this_computer();
    let mut fault = Fault::new(pending.kind);
    fault.program = Some("kernel".to_owned());
    let frame = first_call_trace_frame(lines);
    let stable = frame.unwrap_or_else(|| stable_text(&pending.first));
    fault.signature_text = Some(stable);
    fault
        .detail
        .insert("kernel_message".to_owned(), json!(pending.first));
    if pending.kind == "hung-task" {
        let stuck = stuck_tasks();
        mount_stall_details(&stuck, &mut fault);
        push_text(
            &mut fault.attachments,
            "stuck-tasks.txt",
            scrubber.text(&stuck),
        );
    }
    if pending.kind == "oom" {
        oom_details(lines, &mut fault);
    }
    if pending.kind == "gpu-error" {
        attach_device_dumps(&mut fault);
    }
    let log = lines.iter().cloned().collect::<Vec<_>>().join("\n");
    push_text(&mut fault.attachments, "kernel.txt", scrubber.text(&log));
    let config = Config::load(config_path).unwrap_or_default();
    if let Err(error) = fault.store(spool, &config, spool_root) {
        tracing::warn!("cannot store a kernel fault: {error}");
    }
}

pub fn run(spool_root: &Path, config_path: &Path) -> Result<(), Box<dyn Error>> {
    let spool = Spool::open(spool_root)?;
    let mut kmsg = File::options()
        .read(true)
        .custom_flags((OFlags::NONBLOCK | OFlags::CLOEXEC).bits() as i32)
        .open(KMSG)?;
    let mut seen: u64 = fs::read_to_string(SEEN)
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0);
    let resumed = seen > 0;
    if !resumed {
        kmsg.seek(SeekFrom::Start(0))?;
    }
    let mut lines: VecDeque<String> = VecDeque::with_capacity(KEPT_LINES);
    let mut pending: Option<Pending> = None;
    let mut buffer = vec![0; RECORD_BYTES];
    let pressure = open_pressure_trigger();
    let mut pressed_since: Option<Instant> = None;
    let mut quiet_until: Option<Instant> = None;
    loop {
        if quiet_until.is_some_and(|until| left(until).is_zero()) {
            quiet_until = None;
        }
        let timeout = pending
            .as_ref()
            .map(|pending| pending.since + SETTLE)
            .into_iter()
            .chain(pressed_since.map(|since| since + PRESSURE_SETTLE))
            .chain(quiet_until)
            .min()
            .map(|deadline| {
                let left = left(deadline);
                Timespec {
                    tv_sec: left.as_secs() as i64,
                    tv_nsec: i64::from(left.subsec_nanos()),
                }
            });
        let mut fds = vec![PollFd::new(&kmsg, PollFlags::IN)];
        if let Some(pressure) = pressure.as_ref().filter(|_| quiet_until.is_none()) {
            fds.push(PollFd::new(pressure, PollFlags::PRI));
        }
        match poll(&mut fds, timeout.as_ref()) {
            Ok(_) | Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(error.into()),
        }
        if fds
            .get(1)
            .is_some_and(|fd| fd.revents().contains(PollFlags::PRI))
        {
            pressed_since.get_or_insert_with(Instant::now);
        }
        let before = seen;
        loop {
            match kmsg.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    let line = String::from_utf8_lossy(&buffer[..read]);
                    let Some(record) = parse_record(&line) else {
                        continue;
                    };
                    if record.sequence <= seen && resumed {
                        continue;
                    }
                    seen = record.sequence;
                    if record.level > NOTICE && !record.text.starts_with(OOM_SUMMARY) {
                        continue;
                    }
                    if let Some(name) = expected_event(&record.text) {
                        counters::count(spool_root, name);
                    }
                    if lines.len() == KEPT_LINES {
                        lines.pop_front();
                    }
                    lines.push_back(record.text.clone());
                    if pending.is_none()
                        && let Some(kind) = fault_kind(&record.text)
                    {
                        pending = Some(Pending {
                            kind,
                            first: record.text,
                            since: Instant::now(),
                        });
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error)
                    if error.raw_os_error() == Some(rustix::io::Errno::PIPE.raw_os_error()) => {}
                Err(error) => return Err(error.into()),
            }
        }
        if seen != before
            && let Err(error) = fs::write(SEEN, seen.to_string())
        {
            tracing::debug!("cannot note the last kernel message read: {error}");
        }
        if pending
            .as_ref()
            .is_some_and(|pending| pending.since.elapsed() >= SETTLE)
            && let Some(pending) = pending.take()
        {
            if pending.kind == "oom" {
                pressed_since = None;
            }
            report(&pending, &lines, &spool, config_path, spool_root);
        }
        if pressed_since.is_some_and(|since| since.elapsed() >= PRESSURE_SETTLE)
            && pending.as_ref().is_none_or(|pending| pending.kind != "oom")
        {
            pressed_since = None;
            quiet_until = Some(Instant::now() + PRESSURE_QUIET);
            report_pressure(&spool, config_path, spool_root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{expected_event, fault_kind, parse_record};

    #[test]
    fn kernel_lines_are_sorted_into_faults_and_expected_events() {
        let record = parse_record(
            "3,1234,99,-;INFO: task bande-files:42 blocked for more than 120 seconds.\n SUBSYSTEM=x",
        );
        let Some(record) = record else {
            panic!("a kmsg record");
        };
        assert_eq!((record.level, record.sequence), (3, 1234));
        assert_eq!(fault_kind(&record.text), Some("hung-task"));
        assert_eq!(
            expected_event("CIFS: VFS: \\\\nas has not responded in 180 seconds. Reconnecting"),
            Some("network-server-offline")
        );
        assert_eq!(fault_kind("usb 1-1: new high-speed USB device"), None);
    }
}
