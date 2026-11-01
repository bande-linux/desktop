use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bande_xdg::{DesktopEntry, find_app, unit_app_ids};
use rustix::event::epoll::{self, CreateFlags, EventData, EventFlags};
use zbus::blocking::Connection;

pub const SYSTEM_PRESSURE: &str = "/proc/pressure/memory";
pub const MEMINFO: &str = "/proc/meminfo";
pub const OWN_CGROUP: &str = "/proc/self/cgroup";
pub const CGROUP_ROOT: &str = "/sys/fs/cgroup";

const STALL: Duration = Duration::from_millis(100);
const WINDOW: Duration = Duration::from_secs(2);
const SUSTAINED: Duration = Duration::from_secs(10);
const STREAK_GAP: Duration = Duration::from_secs(5);
const CALM: Duration = Duration::from_secs(60);
const SIGKILL: i32 = 9;
const NEARLY_EXHAUSTED: u64 = 10;

fn trigger(stall: Duration, window: Duration) -> String {
    format!("full {} {}\0", stall.as_micros(), window.as_micros())
}

pub struct PressureTrigger {
    _file: OwnedFd,
    events: OwnedFd,
}

impl PressureTrigger {
    pub fn open(pressure: &Path) -> io::Result<Self> {
        let mut file = OpenOptions::new().read(true).write(true).open(pressure)?;
        file.write_all(trigger(STALL, WINDOW).as_bytes())?;
        let events = epoll::create(CreateFlags::CLOEXEC)?;
        epoll::add(&events, &file, EventData::new_u64(0), EventFlags::PRI)?;
        Ok(Self {
            _file: file.into(),
            events,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.events.as_fd()
    }
}

fn kilobytes(meminfo: &str, key: &str) -> Option<u64> {
    meminfo.lines().find_map(|line| {
        line.strip_prefix(key)?
            .strip_prefix(':')?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

pub fn memory_exhausted(meminfo: &str) -> bool {
    let read = |key| kilobytes(meminfo, key);
    let (Some(total), Some(available)) = (read("MemTotal"), read("MemAvailable")) else {
        return false;
    };
    let swap_total = read("SwapTotal").unwrap_or(0);
    let swap_free = read("SwapFree").unwrap_or(0).min(swap_total);
    (available + swap_free) * NEARLY_EXHAUSTED < total + swap_total
}

#[derive(Debug, Default)]
pub struct Streak {
    started: Option<Instant>,
    last: Option<Instant>,
    alerting: bool,
    snoozed: bool,
}

impl Streak {
    pub fn stalled(&mut self, now: Instant, exhausted: bool) -> bool {
        let quiet = self.last.map(|last| now.saturating_duration_since(last));
        if quiet.is_none_or(|quiet| quiet > STREAK_GAP) {
            self.started = Some(now);
        }
        if quiet.is_none_or(|quiet| quiet >= CALM) {
            self.snoozed = false;
        }
        self.last = Some(now);
        let sustained = self
            .started
            .is_some_and(|started| now.saturating_duration_since(started) >= SUSTAINED);
        let alert = sustained && exhausted && !self.snoozed && !self.alerting;
        self.alerting |= alert;
        alert
    }

    pub fn alerting(&self) -> bool {
        self.alerting
    }

    pub fn dismissed(&mut self) {
        self.alerting = false;
        self.snoozed = true;
    }
}

pub fn app_slice(root: &Path, own_cgroup: &str) -> Option<PathBuf> {
    let path = own_cgroup
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?;
    let components: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let manager = components
        .iter()
        .position(|part| part.starts_with("user@") && part.ends_with(".service"))?;
    let mut slice = root.to_path_buf();
    slice.extend(&components[..=manager]);
    slice.push("app.slice");
    Some(slice)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppMemory {
    pub app_id: String,
    pub name: String,
    pub icon: Option<String>,
    pub units: Vec<String>,
    pub bytes: u64,
}

pub fn footprint(smaps_rollup: &str) -> Option<u64> {
    let anonymous = kilobytes(smaps_rollup, "Pss_Anon")?;
    let swapped = kilobytes(smaps_rollup, "SwapPss").unwrap_or(0);
    Some((anonymous + swapped) * 1024)
}

fn cgroup_footprint(folder: &Path) -> u64 {
    let own: u64 = fs::read_to_string(folder.join("cgroup.procs"))
        .unwrap_or_default()
        .lines()
        .filter_map(|pid| fs::read_to_string(format!("/proc/{}/smaps_rollup", pid.trim())).ok())
        .filter_map(|rollup| footprint(&rollup))
        .sum();
    let nested: u64 = fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|child| child.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|child| cgroup_footprint(&child.path()))
        .sum();
    own + nested
}

pub fn heaviest_apps(app_slice: &Path, entries: &[DesktopEntry], limit: usize) -> Vec<AppMemory> {
    let Ok(units) = fs::read_dir(app_slice) else {
        return Vec::new();
    };
    let mut apps = Vec::<AppMemory>::new();
    let mut positions = HashMap::<String, usize>::new();
    for unit in units.filter_map(Result::ok) {
        let Some(name) = unit.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let ids = unit_app_ids(&name);
        let entry = ids.iter().find_map(|id| find_app(entries, id));
        let Some(app_id) = entry
            .map(|entry| entry.app_id().to_owned())
            .or_else(|| ids.last().cloned())
        else {
            continue;
        };
        let bytes = cgroup_footprint(&unit.path());
        let position = *positions.entry(app_id.clone()).or_insert_with(|| {
            apps.push(AppMemory {
                name: entry.map_or_else(|| app_id.clone(), |entry| entry.name.clone()),
                icon: entry.and_then(|entry| entry.icon.clone()),
                app_id,
                units: Vec::new(),
                bytes: 0,
            });
            apps.len() - 1
        });
        let app = &mut apps[position];
        app.units.push(name);
        app.bytes += bytes;
    }
    apps.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.name.cmp(&right.name))
    });
    apps.truncate(limit);
    apps
}

pub fn force_quit(session: &Connection, app: &AppMemory) -> zbus::Result<()> {
    for unit in &app.units {
        session.call_method(
            Some("org.freedesktop.systemd1"),
            "/org/freedesktop/systemd1",
            Some("org.freedesktop.systemd1.Manager"),
            "KillUnit",
            &(unit, "all", SIGKILL),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustix::event::{PollFd, PollFlags, Timespec, poll};

    #[test]
    fn pressure_trigger_waits_for_a_stall() {
        let Ok(trigger) = PressureTrigger::open(Path::new(SYSTEM_PRESSURE)) else {
            return;
        };
        let fd = trigger.fd();
        let mut fds = [PollFd::new(&fd, PollFlags::IN | PollFlags::PRI)];
        let now = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        assert_eq!(poll(&mut fds, Some(&now)).ok(), Some(0));
    }
}
