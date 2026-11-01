use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use bande_xdg::{
    DesktopEntry, Locale, applications, current_desktops, data_dirs, find_app,
    find_app_for_process, unit_app_ids,
};
use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;

const PROC: &str = "/proc";
const DELETED: &str = " (deleted)";
const PACKAGED: [&str; 2] = ["/usr/", "/opt/"];
const SESSION_SLICE: &str = "session.slice";
const APP_SLICE: &str = "app.slice";
const SESSION_UNITS: [&str; 2] = ["bande-compositor.service", "dbus.service"];

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
trait Systemd {
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

fn replaced(path: &str) -> bool {
    path.strip_suffix(DELETED)
        .is_some_and(|path| PACKAGED.iter().any(|root| path.starts_with(root)))
}

fn runs_replaced_code(executable: &str, maps: &str) -> bool {
    replaced(executable)
        || maps.lines().any(|line| {
            let mut fields = line.splitn(6, ' ');
            let executable = fields.nth(1).is_some_and(|perms| perms.contains('x'));
            executable
                && fields
                    .nth(3)
                    .is_some_and(|path| replaced(path.trim_start()))
        })
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
    let services: BTreeSet<&str> = found
        .iter()
        .filter_map(|(_, holder)| match holder {
            Holder::Service(unit) => Some(unit.as_str()),
            _ => None,
        })
        .collect();
    if !services.is_empty() {
        match SystemdProxyBlocking::new(session) {
            Ok(systemd) => {
                for unit in services {
                    if let Err(error) = systemd.try_restart_unit(unit, "replace") {
                        tracing::warn!("cannot restart {unit}: {error}");
                    }
                }
            }
            Err(error) => tracing::warn!("cannot reach the user's systemd: {error}"),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const MAPS: &str = "\
aaaab0000000-aaaab0100000 r--p 00000000 103:02 4021 /usr/bin/bande-files (deleted)
ffff80000000-ffff80010000 r--p 00000000 103:02 5532                     /usr/share/icons/hicolor/icon-theme.cache (deleted)
ffff80010000-ffff80020000 r-xp 00000000 00:01 77                        /memfd:jit (deleted)
ffff80020000-ffff80030000 r-xp 00000000 103:02 9001                     /home/me/.cargo/bin/tool (deleted)
ffff80030000-ffff80040000 rw-p 00000000 00:00 0
ffff80040000-ffff80050000 r-xp 00010000 103:02 6120                     /usr/lib/aarch64-linux-gnu/libbande widget.so (deleted)
";

    #[test]
    fn only_replaced_packaged_code_counts() {
        let (old_library, rest) = MAPS.split_at(MAPS.rfind("ffff80040000").unwrap_or(0));
        assert!(!runs_replaced_code("/usr/bin/bande-files", old_library));
        assert!(runs_replaced_code("/usr/bin/bande-files", MAPS));
        assert!(runs_replaced_code("/usr/bin/bande-files (deleted)", ""));
        assert!(runs_replaced_code("/usr/bin/bande-files", rest));
    }

    #[test]
    fn processes_group_by_their_unit() {
        let user = "0::/user.slice/user-1000.slice/user@1000.service";
        let held = |unit: &str| holder(&format!("{user}/{unit}\n"));
        assert_eq!(
            held("session.slice/bande-compositor.service"),
            Some(Holder::Session)
        );
        assert_eq!(held("session.slice/dbus.service"), Some(Holder::Session));
        assert_eq!(
            held("session.slice/bande-power.service"),
            Some(Holder::Service("bande-power.service".to_owned()))
        );
        assert_eq!(
            held("app.slice/app-bande-org.bande.Files-1564.scope"),
            Some(Holder::App(
                "app-bande-org.bande.Files-1564.scope".to_owned()
            ))
        );
        assert_eq!(held("background.slice/tracker.service"), None);
        assert_eq!(
            holder("0::/user.slice/user-1000.slice/session-2.scope\n"),
            Some(Holder::Session)
        );
        assert_eq!(holder("0::/system.slice/cron.service\n"), None);
    }
}
