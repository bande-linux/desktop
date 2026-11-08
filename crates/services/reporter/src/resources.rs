use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::checkin;
use crate::config::Config;
use crate::fault::{Fault, push_text, user_stacks};
use crate::hang::Sampler;
use crate::spool::Spool;
use crate::stacks::parse_eu_stack;

const CGROUPS: &str = "/sys/fs/cgroup";
const WATCHED_TREES: [&str; 2] = ["user.slice", "system.slice"];
const SYSTEM_SERVICES: &str = "system.slice/bande-";
const INTERVAL: Duration = Duration::from_secs(10 * 60);
const SLACK: Duration = Duration::from_secs(5 * 60);
const CPU_SHARE: f64 = 0.9;
const CPU_WINDOW: Duration = Duration::from_secs(3 * 60);
const WAKEUPS_PER_SECOND: f64 = 150.0;
const WAKEUP_WINDOW: Duration = Duration::from_secs(5 * 60);
const WRITTEN_PER_DAY: u64 = 2 * 1024 * 1024 * 1024;
const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const SAMPLES: usize = 3;
const SAMPLE_GAP: Duration = Duration::from_secs(1);
const CLOCK_TICKS: f64 = 100.0;
const BANDE_APP_SCOPE: &str = "app-bande-";
const APP_SCOPE: &str = "app-";
const ESCAPED_DASH: &str = "\\x2d";

#[derive(Default, Clone, Copy)]
struct Counts {
    cpu_seconds: f64,
    switches: u64,
    written: u64,
}

#[derive(Default)]
struct Unit {
    counts: Counts,
    processes: HashMap<u32, Counts>,
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn named_number(text: &str, name: &str) -> u64 {
    text.split_whitespace()
        .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
        .or_else(|| {
            text.lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix(' '))
        })
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

fn written_by(io_stat: &str) -> u64 {
    io_stat
        .lines()
        .map(|device| named_number(device, "wbytes"))
        .sum()
}

fn process_counts(pid: u32) -> Option<Counts> {
    let stat = read(Path::new(&format!("/proc/{pid}/stat")));
    let (_, fields) = stat.rsplit_once(") ")?;
    let fields: Vec<&str> = fields.split(' ').collect();
    let ticks: u64 = fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
    let switches = fs::read_dir(format!("/proc/{pid}/task"))
        .ok()?
        .flatten()
        .map(|task| {
            let status = read(&task.path().join("status"));
            ["voluntary_ctxt_switches:", "nonvoluntary_ctxt_switches:"]
                .iter()
                .filter_map(|name| {
                    status
                        .lines()
                        .find_map(|line| line.strip_prefix(name))
                        .and_then(|value| value.trim().parse::<u64>().ok())
                })
                .sum::<u64>()
        })
        .sum();
    let written = named_number(&read(Path::new(&format!("/proc/{pid}/io"))), "write_bytes:");
    Some(Counts {
        cpu_seconds: ticks as f64 / CLOCK_TICKS,
        switches,
        written,
    })
}

fn is_watched(relative: &Path) -> bool {
    let text = relative.to_string_lossy();
    let leaf = text.ends_with(".scope") || text.ends_with(".service");
    leaf && (text.starts_with(WATCHED_TREES[0]) || text.starts_with(SYSTEM_SERVICES))
}

fn sample() -> HashMap<PathBuf, Unit> {
    let root = Path::new(CGROUPS);
    let mut units = HashMap::new();
    let mut folders: Vec<PathBuf> = WATCHED_TREES.iter().map(|tree| root.join(tree)).collect();
    while let Some(folder) = folders.pop() {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                folders.push(entry.path());
            }
        }
        let Ok(relative) = folder.strip_prefix(root) else {
            continue;
        };
        if !is_watched(relative) {
            continue;
        }
        let processes: HashMap<u32, Counts> = read(&folder.join("cgroup.procs"))
            .lines()
            .filter_map(|pid| pid.trim().parse().ok())
            .filter_map(|pid| Some((pid, process_counts(pid)?)))
            .collect();
        let counts = Counts {
            cpu_seconds: named_number(&read(&folder.join("cpu.stat")), "usage_usec") as f64
                / 1_000_000.0,
            switches: processes.values().map(|counts| counts.switches).sum(),
            written: written_by(&read(&folder.join("io.stat"))),
        };
        units.insert(relative.to_path_buf(), Unit { counts, processes });
    }
    units
}

fn busiest(before: &Unit, now: &Unit, measure: impl Fn(&Counts, &Counts) -> f64) -> Option<u32> {
    now.processes
        .iter()
        .filter_map(|(pid, counts)| Some((*pid, measure(before.processes.get(pid)?, counts))))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(pid, _)| pid)
}

fn app_of(unit: &str) -> Option<String> {
    let name = unit.strip_suffix(".scope")?;
    let name = name
        .strip_prefix(BANDE_APP_SCOPE)
        .or_else(|| name.strip_prefix(APP_SCOPE))?;
    let (name, number) = name.rsplit_once('-')?;
    number
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| name.replace(ESCAPED_DASH, "-"))
}

struct Trigger {
    kind: &'static str,
    unit: PathBuf,
    pid: u32,
    figures: serde_json::Map<String, serde_json::Value>,
}

fn report(trigger: Trigger, spool: &Spool, config: &Config, spool_root: &Path) -> Option<String> {
    let pid = trigger.pid.to_string();
    let executable = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    if !bande_log::is_installed_program(&executable) {
        return None;
    }
    let program = executable.file_name()?.to_str()?.to_owned();
    let mut fault = Fault::new(trigger.kind);
    fault.program = Some(program.clone());
    fault.executable = Some(executable);
    fault.pid = Some(pid.clone());
    fault.owner = fs::metadata(format!("/proc/{pid}"))
        .ok()
        .map(|metadata| std::os::unix::fs::MetadataExt::uid(&metadata))
        .filter(|uid| *uid != 0);
    let unit = trigger
        .unit
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    fault.detail.insert("app".to_owned(), json!(app_of(&unit)));
    fault.detail.insert("unit".to_owned(), json!(unit));
    fault.detail.extend(trigger.figures);
    let mut sampler = Sampler::new(trigger.pid);
    for taken in 0..SAMPLES {
        if taken > 0 {
            sleep(SAMPLE_GAP);
        }
        sampler.add(parse_eu_stack(&user_stacks(&pid), u64::from(trigger.pid)));
    }
    if sampler.samples() > 0 {
        let tree = sampler.tree("samples", None, SAMPLE_GAP.as_millis());
        fault.stacks = sampler.heaviest();
        push_text(&mut fault.attachments, "stack-samples.txt", tree);
    }
    if let Err(error) = fault.store(spool, config, spool_root) {
        tracing::warn!("cannot store a resource report: {error}");
    }
    Some(program)
}

pub fn run(spool_root: &Path, config_path: &Path) -> Result<(), Box<dyn Error>> {
    let slack = NonZeroU64::new(u64::try_from(SLACK.as_nanos()).unwrap_or(u64::MAX));
    if let Err(error) = rustix::thread::set_current_timer_slack(slack) {
        tracing::debug!("cannot set the timer slack: {error}");
    }
    let spool = Spool::open(spool_root)?;
    let mut before = sample();
    let mut measured = Instant::now();
    let mut day_started = measured;
    let mut written_today: HashMap<PathBuf, u64> = HashMap::new();
    let mut reported: HashMap<(&'static str, String), Instant> = HashMap::new();
    loop {
        sleep(INTERVAL);
        let config = Config::load(config_path).unwrap_or_default();
        checkin::queue_if_due(&spool, &config);
        let now = sample();
        let at = Instant::now();
        let elapsed = at.duration_since(measured);
        if at.duration_since(day_started) >= DAY {
            day_started = at;
            written_today.clear();
        }
        let seconds = elapsed.as_secs_f64().max(1.0);
        let mut triggers = Vec::new();
        for (path, unit) in &now {
            let Some(previous) = before.get(path) else {
                continue;
            };
            let cpu = (unit.counts.cpu_seconds - previous.counts.cpu_seconds) / seconds;
            let wakeups = unit
                .counts
                .switches
                .saturating_sub(previous.counts.switches) as f64
                / seconds;
            let written = written_today.entry(path.clone()).or_default();
            *written += unit.counts.written.saturating_sub(previous.counts.written);
            let figures = json!({
                "cpu_share": (cpu * 100.0).round() / 100.0,
                "wakeups_per_second": wakeups.round(),
                "written_bytes_today": *written,
                "interval_seconds": elapsed.as_secs(),
            });
            let serde_json::Value::Object(figures) = figures else {
                continue;
            };
            let found = if elapsed >= CPU_WINDOW && cpu > CPU_SHARE {
                Some((
                    "cpu-resource",
                    busiest(previous, unit, |a, b| b.cpu_seconds - a.cpu_seconds),
                ))
            } else if elapsed >= WAKEUP_WINDOW && wakeups > WAKEUPS_PER_SECOND {
                Some((
                    "wakeups-resource",
                    busiest(previous, unit, |a, b| {
                        b.switches.saturating_sub(a.switches) as f64
                    }),
                ))
            } else if *written > WRITTEN_PER_DAY {
                Some((
                    "disk-writes-resource",
                    busiest(previous, unit, |a, b| {
                        b.written.saturating_sub(a.written) as f64
                    }),
                ))
            } else {
                None
            };
            if let Some((kind, Some(pid))) = found {
                triggers.push(Trigger {
                    kind,
                    unit: path.clone(),
                    pid,
                    figures,
                });
            }
        }
        for trigger in triggers {
            let program = fs::read_link(format!("/proc/{}/exe", trigger.pid))
                .ok()
                .and_then(|path| path.file_name()?.to_str().map(str::to_owned))
                .unwrap_or_default();
            let key = (trigger.kind, program);
            if reported.get(&key).is_some_and(|when| when.elapsed() < DAY) {
                continue;
            }
            if report(trigger, &spool, &config, spool_root).is_some() {
                reported.insert(key, Instant::now());
            }
        }
        reported.retain(|_, when| when.elapsed() < DAY);
        before = now;
        measured = at;
    }
}
