use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bande_xdg::{DesktopEntry, find_app, unit_app_ids};
use zbus::blocking::Connection;

pub const SYSTEM_PRESSURE: &str = "/proc/pressure/memory";
pub const OWN_CGROUP: &str = "/proc/self/cgroup";
pub const CGROUP_ROOT: &str = "/sys/fs/cgroup";

const STALL: Duration = Duration::from_millis(100);
const WINDOW: Duration = Duration::from_secs(2);
const SUSTAINED: Duration = Duration::from_secs(10);
const STREAK_GAP: Duration = Duration::from_secs(5);
const CALM: Duration = Duration::from_secs(60);
const SIGKILL: i32 = 9;

fn trigger(stall: Duration, window: Duration) -> String {
    format!("full {} {}\0", stall.as_micros(), window.as_micros())
}

pub struct PressureTrigger {
    file: OwnedFd,
}

impl PressureTrigger {
    pub fn open(pressure: &Path) -> io::Result<Self> {
        let mut file = OpenOptions::new().read(true).write(true).open(pressure)?;
        file.write_all(trigger(STALL, WINDOW).as_bytes())?;
        Ok(Self { file: file.into() })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}

#[derive(Debug, Default)]
pub struct Streak {
    started: Option<Instant>,
    last: Option<Instant>,
    alerting: bool,
    snoozed: bool,
}

impl Streak {
    pub fn stalled(&mut self, now: Instant) -> bool {
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
        let alert = sustained && !self.snoozed && !self.alerting;
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

fn read_bytes(folder: &Path, file: &str) -> u64 {
    fs::read_to_string(folder.join(file))
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
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
        let folder = unit.path();
        let bytes =
            read_bytes(&folder, "memory.current") + read_bytes(&folder, "memory.swap.current");
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
