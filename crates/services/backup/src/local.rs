use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::disk::{self, LOCAL_BROWSE_DIR};

const HOME: &str = "/home";
const SNAPSHOTS: &str = "/.snapshots/home";
const PREFIX: &str = "local-";
const KEEP: usize = 24;
const NAME_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";
const SPACING_MINUTES: i64 = 50;
const LEAST_FREE_SHARE: u64 = 20;

fn is_btrfs_mount(point: &Path) -> bool {
    fs::read_to_string("/proc/self/mounts").is_ok_and(|mounts| {
        mounts.lines().any(|line| {
            let mut fields = line.split(' ').skip(1);
            fields.next().map(Path::new) == Some(point) && fields.next() == Some("btrfs")
        })
    })
}

fn available() -> bool {
    is_btrfs_mount(Path::new(HOME)) && is_btrfs_mount(Path::new("/.snapshots"))
}

fn times() -> Vec<String> {
    let mut times: Vec<String> = fs::read_dir(SNAPSHOTS)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .strip_prefix(PREFIX)
                        .map(str::to_owned)
                })
                .collect()
        })
        .unwrap_or_default();
    times.sort();
    times
}

fn path_of(time: &str) -> PathBuf {
    Path::new(SNAPSHOTS).join(format!("{PREFIX}{time}"))
}

fn delete(time: &str) {
    let deleted = Command::new("btrfs")
        .args(["subvolume", "delete"])
        .arg(path_of(time))
        .stdout(Stdio::null())
        .status();
    if !deleted.is_ok_and(|status| status.success()) {
        tracing::warn!("cannot delete the local snapshot of {time}");
    }
}

fn parse(time: &str) -> Option<jiff::Timestamp> {
    let civil = time.parse::<jiff::civil::DateTime>().ok()?;
    Some(
        civil
            .to_zoned(jiff::tz::TimeZone::system())
            .ok()?
            .timestamp(),
    )
}

fn prune(now: jiff::Timestamp) {
    let mut kept = times();
    while kept.len() > KEEP {
        delete(&kept.remove(0));
    }
    let day_ago = now.as_second() - 24 * 3600;
    while kept
        .first()
        .is_some_and(|time| parse(time).is_some_and(|stamp| stamp.as_second() < day_ago))
    {
        delete(&kept.remove(0));
    }
    while !kept.is_empty()
        && disk::space(Path::new(HOME))
            .is_ok_and(|space| space.available < space.total / LEAST_FREE_SHARE)
    {
        delete(&kept.remove(0));
    }
}

pub fn take_hourly() {
    if !available() {
        return;
    }
    let now = jiff::Zoned::now();
    prune(now.timestamp());
    let recent = times()
        .last()
        .and_then(|time| parse(time))
        .is_some_and(|last| {
            (now.timestamp().as_second() - last.as_second()) < SPACING_MINUTES * 60
        });
    if recent {
        return;
    }
    if let Err(error) = fs::create_dir_all(SNAPSHOTS) {
        tracing::warn!("cannot make {SNAPSHOTS}: {error}");
        return;
    }
    let name = now.strftime(NAME_FORMAT).to_string();
    let taken = Command::new("btrfs")
        .args(["subvolume", "snapshot", "-r", HOME])
        .arg(path_of(&name))
        .stdout(Stdio::null())
        .status();
    if !taken.is_ok_and(|status| status.success()) {
        tracing::warn!("cannot take a local snapshot of {HOME}");
    }
}

pub fn open_view() -> bool {
    let times = times();
    if times.is_empty() {
        return false;
    }
    let root = Path::new(LOCAL_BROWSE_DIR);
    if fs::create_dir_all(root).is_err() {
        return false;
    }
    let tmpfs = Command::new("mount")
        .args(["-t", "tmpfs", "-o", "mode=0755,nosuid,nodev,noexec", "none"])
        .arg(root)
        .status();
    if !tmpfs.is_ok_and(|status| status.success()) {
        return false;
    }
    for time in &times {
        let inside = root.join("versions").join(time).join("home");
        let bound = fs::create_dir_all(&inside).is_ok()
            && Command::new("mount")
                .args(["--bind", "-o", "ro,nosuid,nodev,noexec"])
                .arg(path_of(time))
                .arg(&inside)
                .status()
                .is_ok_and(|status| status.success());
        if !bound {
            tracing::warn!("cannot show the local snapshot of {time}");
        }
    }
    true
}

pub fn close_view() {
    let closed = Command::new("umount")
        .args(["-R", "-l", LOCAL_BROWSE_DIR])
        .stderr(Stdio::null())
        .status();
    if let Err(error) = closed {
        tracing::debug!("{LOCAL_BROWSE_DIR} stays mounted: {error}");
    }
}
