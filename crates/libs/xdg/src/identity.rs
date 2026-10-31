use std::fs;
use std::path::{Path, PathBuf};

use crate::entry::{DesktopEntry, find_executable};
use crate::units::unit_app_ids;

const LAUNCHERS: &[&str] = &[
    "bash", "dash", "env", "fish", "flatpak", "java", "node", "perl", "pkexec", "python",
    "python3", "ruby", "sh", "sudo", "zsh",
];

impl DesktopEntry {
    fn own_program(&self) -> Option<String> {
        if self.terminal {
            return None;
        }
        let program = self.command().ok()?.into_iter().next()?;
        let name = Path::new(&program).file_name()?.to_str()?;
        (!LAUNCHERS.contains(&name)).then_some(program)
    }

    pub fn program(&self) -> Option<String> {
        let program = self.own_program()?;
        Path::new(&program).file_name()?.to_str().map(str::to_owned)
    }

    fn executable(&self) -> Option<PathBuf> {
        let path = find_executable(&self.own_program()?)?;
        Some(fs::canonicalize(&path).unwrap_or(path))
    }
}

fn last_part(id: &str) -> Option<&str> {
    id.rsplit_once('.').map(|(_, last)| last)
}

pub fn find_app<'a>(entries: &'a [DesktopEntry], app_id: &str) -> Option<&'a DesktopEntry> {
    if app_id.is_empty() {
        return None;
    }
    let same = |name: &str| name.eq_ignore_ascii_case(app_id);
    let reverse_dns = |entry: &DesktopEntry| {
        let id = entry.app_id();
        last_part(id).is_some_and(same)
            || (!id.contains('.')
                && last_part(app_id).is_some_and(|last| last.eq_ignore_ascii_case(id)))
    };
    let rules: [&dyn Fn(&DesktopEntry) -> bool; 5] = [
        &|entry| entry.app_id() == app_id,
        &|entry| same(entry.app_id()),
        &|entry| entry.startup_wm_class.as_deref().is_some_and(same),
        &reverse_dns,
        &|entry| entry.program().as_deref().is_some_and(same),
    ];
    rules
        .iter()
        .find_map(|rule| entries.iter().find(|entry| rule(entry)))
}

pub fn app_name(entries: &[DesktopEntry], app_id: &str) -> String {
    find_app(entries, app_id).map_or_else(
        || last_part(app_id).unwrap_or(app_id).to_owned(),
        |entry| entry.name.clone(),
    )
}

pub fn running_app<'a>(
    entries: &[DesktopEntry],
    running: &'a [String],
    app_id: &str,
) -> Option<&'a str> {
    let running = || running.iter().map(String::as_str);
    running().find(|window| *window == app_id).or_else(|| {
        let wanted = find_app(entries, app_id)?.app_id();
        running().find(|window| find_app(entries, window).map(DesktopEntry::app_id) == Some(wanted))
    })
}

fn trusted_scope(cgroup: &str, pid: u32) -> Option<&str> {
    let unit = cgroup
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?
        .rsplit('/')
        .next()?;
    let scope = unit.strip_suffix(".scope")?;
    let own = scope
        .rsplit_once('-')
        .is_some_and(|(_, number)| number == pid.to_string());
    (own || scope.starts_with("app-flatpak-")).then_some(unit)
}

fn process_executable(process: &Path) -> Option<PathBuf> {
    let link = fs::read_link(process.join("exe")).ok()?;
    let text = link.to_str()?;
    let path = PathBuf::from(text.strip_suffix(" (deleted)").unwrap_or(text));
    Some(fs::canonicalize(&path).unwrap_or(path))
}

pub fn find_app_for_process<'a>(
    entries: &'a [DesktopEntry],
    proc_root: &Path,
    pid: u32,
) -> Option<&'a DesktopEntry> {
    let process = proc_root.join(pid.to_string());
    let cgroup = fs::read_to_string(process.join("cgroup")).unwrap_or_default();
    let scoped = trusted_scope(&cgroup, pid)
        .map(unit_app_ids)
        .unwrap_or_default()
        .iter()
        .find_map(|app_id| find_app(entries, app_id));
    scoped.or_else(|| {
        let executable = process_executable(&process)?;
        entries
            .iter()
            .find(|entry| entry.executable().as_ref() == Some(&executable))
    })
}
