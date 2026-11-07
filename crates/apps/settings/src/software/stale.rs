use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use bande_xdg::{
    DesktopEntry, Locale, applications, current_desktops, data_dirs, find_app,
    find_app_for_process, runs_replaced_code, unit_app_ids,
};
use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;

const PROC: &str = "/proc";
const SESSION_SLICE: &str = "session.slice";
const APP_SLICE: &str = "app.slice";
const SESSION_UNITS: [&str; 2] = ["bande-compositor.service", "dbus.service"];

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
trait Systemd {
    fn reload(&self) -> zbus::Result<()>;
    fn try_restart_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Holder {
    Service(String),
    Session,
    App(String),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Stale {
    pub(crate) logout: bool,
    pub(crate) apps: Vec<String>,
}

fn holder(cgroup: &str) -> Option<Holder> {
    let path = cgroup.lines().find_map(|line| line.strip_prefix("0::"))?;
    let mut parts = path.rsplit('/');
    let unit = parts.next()?;
    match parts.next()? {
        SESSION_SLICE if SESSION_UNITS.contains(&unit) => Some(Holder::Session),
        SESSION_SLICE if unit.ends_with(".service") => Some(Holder::Service(unit.to_owned())),
        APP_SLICE => Some(Holder::App(unit.to_owned())),
        _ if unit.starts_with("session-") && unit.ends_with(".scope") => Some(Holder::Session),
        _ => None,
    }
}

fn app_name(entries: &[DesktopEntry], unit: &str, pid: u32) -> Option<String> {
    unit_app_ids(unit)
        .iter()
        .find_map(|app_id| find_app(entries, app_id))
        .or_else(|| find_app_for_process(entries, Path::new(PROC), pid))
        .map(|entry| entry.name.clone())
}

fn holders() -> Vec<(u32, Holder)> {
    let own = Path::new(PROC).join("self");
    let (Ok(own_cgroup), Ok(own_uid)) = (
        fs::read_to_string(own.join("cgroup")),
        fs::metadata(&own).map(|metadata| metadata.uid()),
    ) else {
        return Vec::new();
    };
    let Ok(processes) = fs::read_dir(PROC) else {
        return Vec::new();
    };
    processes
        .flatten()
        .filter_map(|process| {
            let pid = process.file_name().to_str()?.parse::<u32>().ok()?;
            let path = process.path();
            if fs::metadata(&path).ok()?.uid() != own_uid {
                return None;
            }
            let cgroup = fs::read_to_string(path.join("cgroup")).ok()?;
            if cgroup == own_cgroup {
                return None;
            }
            let holder = holder(&cgroup)?;
            let executable = fs::read_link(path.join("exe")).ok()?;
            let maps = fs::read_to_string(path.join("maps")).ok()?;
            runs_replaced_code(&executable.to_string_lossy(), &maps).then_some((pid, holder))
        })
        .collect()
}

pub(crate) fn finish_in_session(session: &Connection) -> Stale {
    let found = holders();
    if !found.is_empty() {
        restart_services(session, &found);
    }
    stale_of(&found)
}

pub(crate) fn look() -> Stale {
    stale_of(&holders())
}

fn restart_services(session: &Connection, found: &[(u32, Holder)]) {
    let systemd = match SystemdProxyBlocking::new(session) {
        Ok(systemd) => systemd,
        Err(error) => {
            tracing::warn!("cannot reach the user's systemd: {error}");
            return;
        }
    };
    if let Err(error) = systemd.reload() {
        tracing::warn!("cannot reload the user's units: {error}");
    }
    let services: BTreeSet<&str> = found
        .iter()
        .filter_map(|(_, holder)| match holder {
            Holder::Service(unit) => Some(unit.as_str()),
            _ => None,
        })
        .collect();
    for unit in services {
        if let Err(error) = systemd.try_restart_unit(unit, "replace") {
            tracing::warn!("cannot restart {unit}: {error}");
        }
    }
}

fn stale_of(found: &[(u32, Holder)]) -> Stale {
    let entries = applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    );
    let apps: BTreeSet<String> = found
        .iter()
        .filter_map(|(pid, holder)| match holder {
            Holder::App(unit) => app_name(&entries, unit, *pid),
            _ => None,
        })
        .collect();
    Stale {
        logout: found.iter().any(|(_, holder)| *holder == Holder::Session),
        apps: apps.into_iter().collect(),
    }
}
