use std::fmt;
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rustix::process::{Pid, Signal, kill_process_group};

pub const FLATHUB: &str = "flathub";
const FLATHUB_REPO: &str = "https://dl.flathub.org/repo/flathub.flatpakrepo";
const PROGRAM: &str = "flatpak";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Missing,
    Cancelled,
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(f, "flatpak is not installed"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Failed(details) => write!(f, "{details}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound => Self::Missing,
            _ => Self::Failed(error.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub id: String,
    pub version: String,
    pub origin: String,
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub id: String,
    pub name: String,
    pub version: String,
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Facts {
    pub version: String,
    pub download: u64,
    pub size: u64,
    pub runtime: Option<String>,
    pub runtime_download: u64,
    pub metadata: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Permission {
    Network,
    AllFiles,
    Home,
    ReadOnlyFiles,
    Folders,
    Devices,
    Audio,
    Bluetooth,
    Printing,
    Ssh,
    X11,
    SessionBus,
    SystemBus,
    Settings,
}

#[derive(Clone, Default)]
pub struct Canceller {
    group: Arc<Mutex<Option<Pid>>>,
    cancelled: Arc<AtomicBool>,
}

impl Canceller {
    pub fn cancel(&self) {
        let group = *self.group.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(group) = group {
            self.cancelled.store(true, Ordering::SeqCst);
            if let Err(error) = kill_process_group(group, Signal::TERM) {
                tracing::warn!("cannot stop flatpak: {error}");
            }
        }
    }

    fn hold(&self, child: &Child) {
        self.cancelled.store(false, Ordering::SeqCst);
        *self.group.lock().unwrap_or_else(PoisonError::into_inner) =
            Pid::from_raw(child.id().cast_signed());
    }

    fn release(&self) -> bool {
        *self.group.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.cancelled.swap(false, Ordering::SeqCst)
    }
}

fn command(subcommand: &str, user: bool) -> Command {
    let mut command = Command::new(PROGRAM);
    command
        .arg(subcommand)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null());
    if user {
        command.arg("--user");
    }
    command
}

fn failure(stderr: &[u8]) -> Error {
    let text = String::from_utf8_lossy(stderr);
    let details = text
        .lines()
        .map(|line| line.trim().trim_start_matches("error:").trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Error::Failed(details)
}

fn output(subcommand: &str, args: &[&str]) -> Result<String, Error> {
    let output = command(subcommand, true).args(args).output()?;
    match output.status.success() {
        true => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        false => Err(failure(&output.stderr)),
    }
}

fn columns(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.split('\t').map(str::trim).collect())
}

fn percent(line: &str) -> Option<u32> {
    let step = line
        .split_whitespace()
        .find_map(|word| {
            let (done, total) = word.trim_end_matches('…').split_once('/')?;
            Some((done.parse::<u32>().ok()?, total.parse::<u32>().ok()?))
        })
        .filter(|(done, total)| *done > 0 && done <= total);
    let within = line
        .split_whitespace()
        .rev()
        .find_map(|word| word.strip_suffix('%')?.parse::<u32>().ok())?
        .min(100);
    Some(match step {
        Some((done, total)) => ((done - 1) * 100 + within) / total,
        None => within,
    })
}

fn transact(
    subcommand: &str,
    args: &[&str],
    progress: &dyn Fn(Option<u32>),
    canceller: &Canceller,
) -> Result<(), Error> {
    let mut child = command(subcommand, true)
        .arg("-y")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    canceller.hold(&child);
    let mut stderr = child.stderr.take();
    let errors = std::thread::spawn(move || {
        let mut text = Vec::new();
        if let Some(stderr) = &mut stderr
            && let Err(error) = stderr.read_to_end(&mut text)
        {
            tracing::debug!("cannot read flatpak's errors: {error}");
        }
        text
    });
    if let Some(mut stdout) = child.stdout.take() {
        let mut buffer = [0u8; 4096];
        let mut line = Vec::new();
        loop {
            let read = match stdout.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            for byte in buffer.iter().take(read) {
                if matches!(byte, b'\n' | b'\r') {
                    if let Some(percent) = percent(&String::from_utf8_lossy(&line)) {
                        progress(Some(percent));
                    }
                    line.clear();
                } else {
                    line.push(*byte);
                }
            }
        }
    }
    let status = child.wait();
    let cancelled = canceller.release();
    let stderr = errors.join().unwrap_or_default();
    if cancelled {
        return Err(Error::Cancelled);
    }
    match status?.success() {
        true => Ok(()),
        false => Err(failure(&stderr)),
    }
}

pub fn remotes() -> Result<Vec<String>, Error> {
    Ok(columns(&output("remotes", &["--columns=name"])?)
        .filter_map(|row| row.first().map(|name| (*name).to_owned()))
        .collect())
}

pub fn has_remotes() -> bool {
    remotes().is_ok_and(|remotes| !remotes.is_empty())
}

pub fn enable_flathub() -> Result<(), Error> {
    output("remote-add", &["--if-not-exists", FLATHUB, FLATHUB_REPO])?;
    output("remote-modify", &["--enable", FLATHUB])?;
    output("update", &["--appstream", "--noninteractive", FLATHUB]).map(drop)
}

pub fn disable_flathub() -> Result<(), Error> {
    let known = output("remotes", &["--show-disabled", "--columns=name"])?;
    if !columns(&known).any(|row| row.first() == Some(&FLATHUB)) {
        return Ok(());
    }
    output("remote-modify", &["--disable", FLATHUB]).map(drop)
}

pub fn refresh_catalogue() -> Result<(), Error> {
    if !has_remotes() {
        return Ok(());
    }
    output("update", &["--appstream", "--noninteractive"]).map(drop)
}

pub fn installed() -> Result<Vec<Installed>, Error> {
    let listed = output(
        "list",
        &["--app", "--columns=application,version,origin,ref"],
    )?;
    Ok(columns(&listed)
        .filter_map(|row| match row.as_slice() {
            [id, version, origin, reference, ..] => Some(Installed {
                id: (*id).to_owned(),
                version: (*version).to_owned(),
                origin: (*origin).to_owned(),
                reference: format!("app/{reference}"),
            }),
            _ => None,
        })
        .collect())
}

pub fn updates() -> Result<Vec<Update>, Error> {
    if !has_remotes() {
        return Ok(Vec::new());
    }
    let listed = output(
        "remote-ls",
        &["--updates", "--columns=application,name,version,ref"],
    )?;
    Ok(columns(&listed)
        .filter_map(|row| match row.as_slice() {
            [id, name, version, reference, ..] => Some(Update {
                id: (*id).to_owned(),
                name: (*name).to_owned(),
                version: (*version).to_owned(),
                reference: (*reference).to_owned(),
            }),
            _ => None,
        })
        .collect())
}

fn size(text: &str) -> u64 {
    let mut words = text.split_whitespace();
    let number: f64 = words
        .next()
        .and_then(|number| number.replace(',', ".").parse().ok())
        .unwrap_or(0.0);
    let unit = match words.next().unwrap_or("") {
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        _ => 1.0,
    };
    (number * unit) as u64
}

fn field<'a>(info: &'a str, name: &str) -> Option<&'a str> {
    info.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == name).then(|| value.trim())
    })
}

fn installed_anywhere(reference: &str) -> bool {
    command("info", false)
        .arg(reference)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub fn facts(remote: &str, reference: &str, installed: bool) -> Result<Facts, Error> {
    let (info, metadata) = match installed {
        true => (
            output("info", &[reference])?,
            output("info", &["--show-metadata", reference])?,
        ),
        false => (
            output("remote-info", &[remote, reference])?,
            output("remote-info", &["--show-metadata", remote, reference])?,
        ),
    };
    let runtime = field(&info, "Runtime")
        .map(|runtime| format!("runtime/{runtime}"))
        .filter(|runtime| !installed && !installed_anywhere(runtime));
    let runtime_download = runtime
        .as_deref()
        .and_then(|runtime| output("remote-info", &[remote, runtime]).ok())
        .and_then(|info| field(&info, "Download Size").map(size))
        .unwrap_or(0);
    Ok(Facts {
        version: field(&info, "Version").unwrap_or_default().to_owned(),
        download: field(&info, "Download Size").map(size).unwrap_or(0),
        size: field(&info, "Installed Size").map(size).unwrap_or(0),
        runtime: runtime
            .as_deref()
            .and_then(|runtime| runtime.split('/').nth(1))
            .map(str::to_owned),
        runtime_download,
        metadata,
    })
}

pub fn install(
    remote: &str,
    reference: &str,
    progress: &dyn Fn(Option<u32>),
    canceller: &Canceller,
) -> Result<(), Error> {
    transact("install", &[remote, reference], progress, canceller)
}

pub fn uninstall(
    reference: &str,
    progress: &dyn Fn(Option<u32>),
    canceller: &Canceller,
) -> Result<(), Error> {
    transact("uninstall", &[reference], progress, canceller)
}

pub fn uninstall_unused() -> Result<(), Error> {
    output("uninstall", &["--unused", "-y", "--noninteractive"]).map(drop)
}

pub fn update(
    references: &[String],
    download_only: bool,
    progress: &dyn Fn(Option<u32>),
    canceller: &Canceller,
) -> Result<(), Error> {
    let mut args: Vec<&str> = references.iter().map(String::as_str).collect();
    if download_only {
        args.push("--no-deploy");
    }
    transact("update", &args, progress, canceller)
}

fn context(metadata: &str, section: &str) -> Vec<(String, Vec<String>)> {
    let mut inside = false;
    let mut found = Vec::new();
    for line in metadata.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == section;
        } else if inside && let Some((key, value)) = line.split_once('=') {
            let values = value
                .split(';')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect();
            found.push((key.trim().to_owned(), values));
        }
    }
    found
}

fn file_permission(path: &str) -> Option<Permission> {
    let (path, mode) = path.split_once(':').unwrap_or((path, "rw"));
    let read_only = mode == "ro";
    let permission = match path {
        "host" | "host-os" | "host-etc" | "/" => Permission::AllFiles,
        "home" | "~" => Permission::Home,
        path if path.starts_with("xdg-run") || path.starts_with("xdg-cache") => return None,
        path if path.starts_with("xdg-config") => Permission::Settings,
        _ => Permission::Folders,
    };
    Some(match (permission, read_only) {
        (Permission::AllFiles | Permission::Home, true) => Permission::ReadOnlyFiles,
        (permission, _) => permission,
    })
}

pub fn permissions(metadata: &str) -> Vec<Permission> {
    let mut found = Vec::new();
    for (key, values) in context(metadata, "[Context]") {
        for value in values.iter().filter(|value| !value.starts_with('!')) {
            let permission = match (key.as_str(), value.as_str()) {
                ("shared", "network") => Some(Permission::Network),
                ("sockets", "x11") => Some(Permission::X11),
                ("sockets", "pulseaudio") => Some(Permission::Audio),
                ("sockets", "cups") => Some(Permission::Printing),
                ("sockets", "ssh-auth") => Some(Permission::Ssh),
                ("sockets", "session-bus") => Some(Permission::SessionBus),
                ("sockets", "system-bus") => Some(Permission::SystemBus),
                ("devices", "all" | "input" | "usb" | "kvm" | "shm") => Some(Permission::Devices),
                ("features", "bluetooth") => Some(Permission::Bluetooth),
                ("filesystems", path) => file_permission(path),
                _ => None,
            };
            found.extend(permission);
        }
    }
    if !context(metadata, "[System Bus Policy]").is_empty() {
        found.push(Permission::SystemBus);
    }
    let sockets: Vec<String> = context(metadata, "[Context]")
        .into_iter()
        .filter(|(key, _)| key == "sockets")
        .flat_map(|(_, values)| values)
        .collect();
    if sockets
        .iter()
        .any(|socket| socket == "wayland" || socket == "fallback-x11")
    {
        found.retain(|permission| *permission != Permission::X11);
    }
    found.sort();
    found.dedup();
    if found.contains(&Permission::AllFiles) {
        found.retain(|permission| {
            !matches!(
                permission,
                Permission::Home | Permission::Folders | Permission::ReadOnlyFiles
            )
        });
    } else if found.contains(&Permission::Home) {
        found.retain(|permission| !matches!(permission, Permission::Folders));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::{Permission, percent, permissions};

    #[test]
    fn progress_across_several_refs_and_permissions_from_metadata() {
        assert_eq!(percent("Installing 3/4… ████  50%  1.2 MB/s"), Some(62));
        assert_eq!(percent("Installing… ███ 8%"), Some(8));
        assert_eq!(percent("Installing runtime/org.example/aarch64/1"), None);
        let metadata = "[Application]\nname=a\n\n[Context]\nshared=network;ipc;\n\
            sockets=x11;wayland;pulseaudio;\ndevices=dri;\nfilesystems=home;xdg-download;xdg-run/pipewire-0;\n";
        assert_eq!(
            permissions(metadata),
            vec![Permission::Network, Permission::Home, Permission::Audio]
        );
    }
}
