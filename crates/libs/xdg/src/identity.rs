use std::fs;
use std::io::{self, Write};
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

fn exec_argument(argument: &str) -> String {
    let mut quoted = String::from("\"");
    for character in argument.chars() {
        match character {
            '"' | '`' | '$' | '\\' => {
                quoted.push('\\');
                quoted.push(character);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted.replace('\\', "\\\\")
}

fn is_interpreter(executable: &Path) -> bool {
    executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            let unversioned = name
                .trim_end_matches(|character: char| character.is_ascii_digit() || character == '.');
            LAUNCHERS.contains(&name) || LAUNCHERS.contains(&unversioned)
        })
}

fn script(process: &Path) -> Option<PathBuf> {
    let cmdline = fs::read(process.join("cmdline")).ok()?;
    let script = cmdline
        .split(|byte| *byte == 0)
        .skip(1)
        .filter_map(|argument| std::str::from_utf8(argument).ok())
        .find(|argument| !argument.is_empty() && !argument.starts_with('-'))?;
    Some(
        fs::read_link(process.join("cwd"))
            .map(|cwd| cwd.join(script))
            .unwrap_or_else(|_| PathBuf::from(script)),
    )
}

pub fn write_launcher(
    data_home: &Path,
    app_id: &str,
    proc_root: &Path,
    pid: u32,
) -> io::Result<PathBuf> {
    let unknown = || io::Error::new(io::ErrorKind::NotFound, "the app's program is unknown");
    if app_id.is_empty() || app_id.contains(['/', '\n']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no usable app id",
        ));
    }
    let process = proc_root.join(pid.to_string());
    let executable = process_executable(&process).ok_or_else(unknown)?;
    let program = executable.to_str().ok_or_else(unknown)?;
    let mut command = vec![exec_argument(program)];
    if is_interpreter(&executable) {
        let script = script(&process).ok_or_else(unknown)?;
        command.push(exec_argument(script.to_str().ok_or_else(unknown)?));
    }
    let folder = data_home.join("applications");
    fs::create_dir_all(&folder)?;
    let path = folder.join(format!("{app_id}.desktop"));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let name = last_part(app_id).unwrap_or(app_id);
    write!(
        file,
        "[Desktop Entry]\nType=Application\nName={name}\nExec={}\nTryExec={}\nIcon={app_id}\nStartupWMClass={app_id}\nX-Bande-Kept=true\n",
        command.join(" "),
        program.replace('\\', "\\\\"),
    )?;
    Ok(path)
}
