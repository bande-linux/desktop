use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use unbound_xdg::{DesktopEntry, find_app, unit_app_ids};
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn the_kernel_accepts_the_trigger_from_an_unprivileged_session() {
        if !Path::new(SYSTEM_PRESSURE).exists() {
            return;
        }
        assert!(PressureTrigger::open(Path::new(SYSTEM_PRESSURE)).is_ok());
    }

    #[test]
    fn only_a_stall_that_lasts_raises_the_alert_and_a_dismissal_holds_until_memory_is_calm() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut streak = Streak::default();

        assert!(!streak.stalled(at(0)));
        assert!(!streak.stalled(at(4)));
        assert!(!streak.stalled(at(12)));
        assert!(!streak.stalled(at(14)));
        assert!(!streak.stalled(at(18)));
        assert!(streak.stalled(at(22)));
        assert!(streak.alerting());
        assert!(!streak.stalled(at(24)));

        streak.dismissed();
        assert!(!streak.alerting());
        assert!(!streak.stalled(at(40)));
        assert!(!streak.stalled(at(52)));
        assert!(!streak.alerting());

        assert!(!streak.stalled(at(120)));
        assert!(!streak.stalled(at(124)));
        assert!(!streak.stalled(at(128)));
        assert!(streak.stalled(at(130)));
    }

    #[test]
    fn the_app_slice_belongs_to_the_session_user_manager() {
        let root = Path::new("/sys/fs/cgroup");
        assert_eq!(
            app_slice(
                root,
                "0::/user.slice/user-1000.slice/user@1000.service/session.slice/unbound-compositor.service\n"
            ),
            Some(root.join("user.slice/user-1000.slice/user@1000.service/app.slice"))
        );
        assert_eq!(
            app_slice(root, "0::/user.slice/user-1000.slice/session-3.scope\n"),
            None
        );
    }

    fn entry(id: &str, name: &str) -> DesktopEntry {
        let text =
            format!("[Desktop Entry]\nType=Application\nName={name}\nIcon={id}\nExec={name}\n");
        DesktopEntry::parse(id, Path::new(id), &text, None, &[]).unwrap()
    }

    fn unit(slice: &Path, name: &str, current: u64, swap: Option<u64>) {
        let folder = slice.join(name);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("memory.current"), format!("{current}\n")).unwrap();
        if let Some(swap) = swap {
            fs::write(folder.join("memory.swap.current"), format!("{swap}\n")).unwrap();
        }
    }

    #[test]
    fn the_heaviest_apps_add_up_their_scopes_and_swap() {
        let folder = tempfile::tempdir().unwrap();
        let entries = [
            entry("org.example.Browser", "Browser"),
            entry("org.example.Editor", "Editor"),
        ];
        let slice = folder.path().join("app.slice");
        unit(
            &slice,
            "app-unbound-org.example.Browser-10.scope",
            900,
            Some(300),
        );
        unit(
            &slice,
            "app-unbound-org.example.Browser-11.scope",
            500,
            None,
        );
        unit(
            &slice,
            "app-unbound-org.example.Editor-12.scope",
            1_000,
            Some(0),
        );
        unit(&slice, "app-org.example.Tool@autostart.service", 50, None);
        unit(&slice, "dbus.socket", 1_000_000, None);
        unit(&slice, "app-unbound-org.example.Tiny-13.scope", 10, None);

        let apps = heaviest_apps(&slice, &entries, 3);

        let summary: Vec<(&str, &str, u64, usize)> = apps
            .iter()
            .map(|app| {
                (
                    app.app_id.as_str(),
                    app.name.as_str(),
                    app.bytes,
                    app.units.len(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("org.example.Browser", "Browser", 1_700, 2),
                ("org.example.Editor", "Editor", 1_000, 1),
                ("org.example.Tool", "org.example.Tool", 50, 1),
            ]
        );
        assert_eq!(apps[0].icon.as_deref(), Some("org.example.Browser"));
    }
}
