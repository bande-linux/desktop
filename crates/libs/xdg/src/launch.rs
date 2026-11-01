use std::collections::HashMap;
use std::fmt;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::slice;
use std::sync::LazyLock;
use std::thread;
use std::time::{Duration, Instant};

use bande_inbox::{Inbox, Outbox};
use bande_notify::Urgency;
use bande_notify::client::{Request, notify};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::{Mode, OFlags};
use rustix::io::Errno;
use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open};
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{OwnedValue, Value};
use zbus::{MatchRule, message};

use crate::L10N;
use crate::entry::{
    DesktopAction, DesktopEntry, Locale, applications, current_desktops, data_dirs, find_executable,
};
use crate::identity::{app_name, find_app};
use crate::units::{scope_name, unit_app_ids, unit_of_object_path};
use crate::uri::file_uri;

pub const TERMINAL: &str = "bande-terminal";
pub const ACTIVATION_TOKEN: &str = "XDG_ACTIVATION_TOKEN";
const APP_OOM_SCORE: &[u8] = b"200";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    UnterminatedQuote,
    ReservedCharacter(char),
    Empty,
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnterminatedQuote => write!(f, "the Exec key has an unterminated quote"),
            Self::ReservedCharacter(character) => {
                write!(
                    f,
                    "the Exec key has an unquoted reserved character {character:?}"
                )
            }
            Self::Empty => write!(f, "the Exec key names no program"),
        }
    }
}

impl std::error::Error for ExecError {}

const RESERVED: &[char] = &[
    '\t', '\n', '\'', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(', ')', '`',
];

fn split(exec: &str) -> Result<Vec<(String, bool)>, ExecError> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quoted_argument = false;
    let mut started = false;
    let mut chars = exec.chars();
    while let Some(character) = chars.next() {
        match character {
            ' ' => {
                if started {
                    arguments.push((std::mem::take(&mut current), quoted_argument));
                    started = false;
                    quoted_argument = false;
                }
            }
            '"' => {
                started = true;
                quoted_argument = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(escaped @ ('"' | '`' | '$' | '\\')) => current.push(escaped),
                            Some(other) => {
                                current.push('\\');
                                current.push(other);
                            }
                            None => return Err(ExecError::UnterminatedQuote),
                        },
                        Some(other) => current.push(other),
                        None => return Err(ExecError::UnterminatedQuote),
                    }
                }
            }
            reserved if RESERVED.contains(&reserved) => {
                return Err(ExecError::ReservedCharacter(reserved));
            }
            other => {
                started = true;
                current.push(other);
            }
        }
    }
    if started {
        arguments.push((current, quoted_argument));
    }
    Ok(arguments)
}

const SINGLE_FILE_CODES: [&str; 2] = ["%f", "%u"];
const FILE_LIST_CODES: [&str; 2] = ["%F", "%U"];

fn file_argument(code: &str, file: &Path) -> String {
    if code.eq_ignore_ascii_case("%u") {
        file_uri(file)
    } else {
        file.to_string_lossy().into_owned()
    }
}

impl DesktopEntry {
    pub fn command(&self) -> Result<Vec<String>, ExecError> {
        self.expand(&split(&self.exec)?, &[])
    }

    pub fn action_command(&self, action: &DesktopAction) -> Result<Vec<String>, ExecError> {
        self.expand(&split(&action.exec)?, &[])
    }

    pub fn opens_files(&self) -> bool {
        split(&self.exec).is_ok_and(|arguments| {
            arguments.iter().any(|(argument, quoted)| {
                !quoted
                    && (SINGLE_FILE_CODES.contains(&argument.as_str())
                        || FILE_LIST_CODES.contains(&argument.as_str()))
            })
        })
    }

    pub fn commands_for(&self, files: &[PathBuf]) -> Result<Vec<Vec<String>>, ExecError> {
        let arguments = split(&self.exec)?;
        let has = |codes: [&str; 2]| {
            arguments
                .iter()
                .any(|(argument, quoted)| !quoted && codes.contains(&argument.as_str()))
        };
        if has(FILE_LIST_CODES) || files.is_empty() {
            return Ok(vec![self.expand(&arguments, files)?]);
        }
        if has(SINGLE_FILE_CODES) {
            return files
                .iter()
                .map(|file| self.expand(&arguments, slice::from_ref(file)))
                .collect();
        }
        Ok(vec![self.expand(&arguments, &[])?])
    }

    fn expand(
        &self,
        arguments: &[(String, bool)],
        files: &[PathBuf],
    ) -> Result<Vec<String>, ExecError> {
        let mut command = Vec::new();
        for (argument, quoted) in arguments {
            if *quoted {
                command.push(argument.clone());
                continue;
            }
            match argument.as_str() {
                code @ ("%f" | "%u") => {
                    command.extend(files.first().map(|file| file_argument(code, file)));
                }
                code @ ("%F" | "%U") => {
                    command.extend(files.iter().map(|file| file_argument(code, file)));
                }
                "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => {}
                "%i" => {
                    if let Some(icon) = &self.icon {
                        command.push("--icon".to_owned());
                        command.push(icon.clone());
                    }
                }
                _ => {
                    let mut expanded = String::new();
                    let mut chars = argument.chars();
                    while let Some(character) = chars.next() {
                        if character != '%' {
                            expanded.push(character);
                            continue;
                        }
                        match chars.next() {
                            Some('%') => expanded.push('%'),
                            Some('c') => expanded.push_str(&self.name),
                            Some('k') => expanded.push_str(&self.path.to_string_lossy()),
                            _ => {}
                        }
                    }
                    command.push(expanded);
                }
            }
        }
        if command.is_empty() {
            return Err(ExecError::Empty);
        }
        if self.terminal {
            command.splice(0..0, [TERMINAL.to_owned(), "-e".to_owned()]);
        }
        Ok(command)
    }
}

#[derive(Debug)]
pub enum LaunchError {
    Exec(ExecError),
    NoTerminal,
    Spawn(io::Error),
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exec(error) => write!(f, "{error}"),
            Self::NoTerminal => write!(
                f,
                "the app runs in a terminal and {TERMINAL} is not installed"
            ),
            Self::Spawn(error) => write!(f, "cannot start the app: {error}"),
        }
    }
}

impl std::error::Error for LaunchError {}

pub fn launch_with(
    entry: &DesktopEntry,
    files: &[PathBuf],
    activation_token: Option<&str>,
) -> Result<(), LaunchError> {
    for command in entry.commands_for(files).map_err(LaunchError::Exec)? {
        start(entry, &command, activation_token)?;
    }
    Ok(())
}

pub fn launch(entry: &DesktopEntry, activation_token: Option<&str>) -> Result<u32, LaunchError> {
    launch_with_arguments(entry, &[], activation_token)
}

pub fn launch_with_arguments(
    entry: &DesktopEntry,
    arguments: &[String],
    activation_token: Option<&str>,
) -> Result<u32, LaunchError> {
    let mut command = entry.command().map_err(LaunchError::Exec)?;
    command.extend_from_slice(arguments);
    start(entry, &command, activation_token)
}

pub fn launch_action(
    entry: &DesktopEntry,
    action: &DesktopAction,
    activation_token: Option<&str>,
) -> Result<u32, LaunchError> {
    let command = entry.action_command(action).map_err(LaunchError::Exec)?;
    start(entry, &command, activation_token)
}

fn start(
    entry: &DesktopEntry,
    command: &[String],
    activation_token: Option<&str>,
) -> Result<u32, LaunchError> {
    if entry.terminal && find_executable(TERMINAL).is_none() {
        return Err(LaunchError::NoTerminal);
    }
    spawn_detached(
        entry.app_id(),
        command,
        entry.working_directory.as_deref(),
        activation_token,
    )
    .map_err(LaunchError::Spawn)
}

pub fn wait_for_exit(
    child: &mut Child,
    deadline: Option<Instant>,
    stop: Option<BorrowedFd<'_>>,
) -> io::Result<Option<ExitStatus>> {
    let exited = pidfd_open(Pid::from_child(child), PidfdFlags::empty())?;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        let timeout = match deadline {
            Some(deadline) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Ok(None);
                }
                Some(Timespec {
                    tv_sec: i64::try_from(left.as_secs()).unwrap_or(i64::MAX),
                    tv_nsec: i64::from(left.subsec_nanos()),
                })
            }
            None => None,
        };
        let mut fds: Vec<PollFd<'_>> = [exited.as_fd()]
            .into_iter()
            .chain(stop)
            .map(|fd| PollFd::from_borrowed_fd(fd, PollFlags::IN))
            .collect();
        match poll(&mut fds, timeout.as_ref()) {
            Ok(_) | Err(Errno::INTR) => {}
            Err(error) => return Err(error.into()),
        }
        if fds.get(1).is_some_and(|fd| !fd.revents().is_empty()) {
            return Ok(None);
        }
    }
}

pub fn spawn_detached(
    app_id: &str,
    command: &[String],
    working_directory: Option<&Path>,
    activation_token: Option<&str>,
) -> io::Result<u32> {
    let (program, arguments) = command
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no program"))?;
    let mut process = Command::new(program);
    process
        .args(arguments)
        .stdin(Stdio::null())
        .env_remove(ACTIVATION_TOKEN);
    if let Some(token) = activation_token {
        process.env(ACTIVATION_TOKEN, token);
    }
    if let Some(directory) = working_directory {
        process.current_dir(directory);
    }
    unsafe {
        process.pre_exec(|| {
            rustix::process::setsid().map_err(io::Error::from)?;
            if let Ok(score) = rustix::fs::open(
                c"/proc/self/oom_score_adj",
                OFlags::WRONLY | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                let _ = rustix::io::write(&score, APP_OOM_SCORE);
            }
            Ok(())
        });
    }
    let app = (!app_id.is_empty()).then(|| Supervised {
        app_id: app_id.to_owned(),
        started: Instant::now(),
    });
    match process.spawn() {
        Ok(child) => {
            let pid = child.id();
            watch(Watch::Child(child, app));
            Ok(pid)
        }
        Err(error) => {
            if let Some(app) = app {
                watch(Watch::NotOpened(app.app_id));
            }
            Err(error)
        }
    }
}

struct Supervised {
    app_id: String,
    started: Instant,
}

enum Watch {
    Child(Child, Option<Supervised>),
    NotOpened(String),
}

struct Watched {
    child: Child,
    exited: OwnedFd,
    app: Option<Supervised>,
}

static WATCHER: LazyLock<Option<Outbox<Watch>>> = LazyLock::new(|| {
    let started = Inbox::new().and_then(|inbox| {
        let outbox = inbox.outbox();
        thread::Builder::new()
            .name("app-watcher".to_owned())
            .spawn(move || Watcher::new(bande_bus::session).run(inbox))?;
        Ok(outbox)
    });
    started
        .inspect_err(|error| tracing::warn!("launched apps are not followed: {error}"))
        .ok()
});

fn watch(watch: Watch) {
    if !WATCHER.as_ref().is_some_and(|watcher| watcher.send(watch)) {
        tracing::warn!("a launched app is not followed");
    }
}

struct Watcher<C> {
    connect: C,
    bus: Option<Connection>,
    watched: Vec<Watched>,
}

impl<C: Fn() -> zbus::Result<Connection>> Watcher<C> {
    fn new(connect: C) -> Self {
        Self {
            connect,
            bus: None,
            watched: Vec::new(),
        }
    }

    fn run(mut self, mut inbox: Inbox<Watch>) {
        loop {
            match self.wait(&inbox) {
                Ok(ended) => self.finish(ended),
                Err(error) => {
                    return tracing::warn!("launched apps are no longer followed: {error}");
                }
            }
            for watch in inbox.drain() {
                self.take(watch);
            }
        }
    }

    fn wait(&self, inbox: &Inbox<Watch>) -> rustix::io::Result<Vec<usize>> {
        let ready = inbox.fd();
        let mut fds: Vec<PollFd<'_>> = std::iter::once(PollFd::new(&ready, PollFlags::IN))
            .chain(
                self.watched
                    .iter()
                    .map(|watched| PollFd::new(&watched.exited, PollFlags::IN)),
            )
            .collect();
        match poll(&mut fds, None) {
            Ok(_) => {}
            Err(Errno::INTR) => return Ok(Vec::new()),
            Err(error) => return Err(error),
        }
        Ok(fds
            .iter()
            .skip(1)
            .enumerate()
            .filter(|(_, fd)| !fd.revents().is_empty())
            .map(|(index, _)| index)
            .collect())
    }

    fn bus(&mut self) -> zbus::Result<Connection> {
        let bus = match self.bus.take() {
            Some(bus) => bus,
            None => (self.connect)()?,
        };
        Ok(self.bus.insert(bus).clone())
    }

    fn take(&mut self, watch: Watch) {
        let (child, app) = match watch {
            Watch::NotOpened(app_id) => return tell(self.bus(), &app_id, Failure::NotOpened),
            Watch::Child(child, app) => (child, app),
        };
        if let Some(app) = &app {
            let placed = self
                .bus()
                .and_then(|bus| place_in_scope(&bus, &app.app_id, child.id()));
            if let Err(error) = placed {
                tracing::debug!("{} runs outside its own scope: {error}", app.app_id);
            }
        }
        match pidfd_open(Pid::from_child(&child), PidfdFlags::empty()) {
            Ok(exited) => self.watched.push(Watched { child, exited, app }),
            Err(error) => {
                tracing::debug!("cannot follow {}: {error}", child.id());
                let mut child = child;
                if let Err(error) = thread::Builder::new().spawn(move || child.wait()) {
                    tracing::warn!("a launched app is not followed: {error}");
                }
            }
        }
    }

    fn finish(&mut self, ended: Vec<usize>) {
        for index in ended.into_iter().rev() {
            let Watched { mut child, app, .. } = self.watched.remove(index);
            let status = match child.wait() {
                Ok(status) => status,
                Err(error) => {
                    tracing::debug!("cannot follow {}: {error}", child.id());
                    continue;
                }
            };
            if let Some(app) = app
                && let Some(failure) = failure(status, app.started.elapsed())
            {
                tracing::warn!("{} ended with {status}", app.app_id);
                tell(self.bus(), &app.app_id, failure);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    NotOpened,
    Crashed,
    OutOfMemory,
}

const OPENING: Duration = Duration::from_secs(5);
const CRASHES: [Signal; 6] = [
    Signal::SEGV,
    Signal::ABORT,
    Signal::BUS,
    Signal::ILL,
    Signal::FPE,
    Signal::SYS,
];
const SYSTEMD: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const SYSTEMD_MANAGER: &str = "org.freedesktop.systemd1.Manager";
const UNIT_INTERFACES: [&str; 2] = [
    "org.freedesktop.systemd1.Scope",
    "org.freedesktop.systemd1.Service",
];
const OOM_KILL: &str = "oom-kill";

fn failure(status: ExitStatus, ran: Duration) -> Option<Failure> {
    if let Some(signal) = status.signal() {
        return CRASHES
            .iter()
            .any(|crash| crash.as_raw() == signal)
            .then_some(Failure::Crashed);
    }
    (!status.success() && ran < OPENING).then_some(Failure::NotOpened)
}

fn place_in_scope(bus: &Connection, app_id: &str, pid: u32) -> zbus::Result<()> {
    let properties = vec![
        ("PIDs", Value::from(vec![pid])),
        ("Slice", Value::from("app.slice")),
        ("CollectMode", Value::from("inactive-or-failed")),
    ];
    bus.call_method(
        Some(SYSTEMD),
        SYSTEMD_PATH,
        Some(SYSTEMD_MANAGER),
        "StartTransientUnit",
        &(
            scope_name(app_id, pid),
            "fail",
            properties,
            Vec::<(&str, Vec<(&str, Value)>)>::new(),
        ),
    )?;
    Ok(())
}

fn tell(bus: zbus::Result<Connection>, app_id: &str, failure: Failure) {
    let entries = applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    );
    let entry = find_app(&entries, app_id);
    let app = app_name(&entries, app_id);
    let (summary, body) = match failure {
        Failure::NotOpened => ("app-not-opened", "app-not-opened-body"),
        Failure::Crashed => ("app-crashed", "app-crashed-body"),
        Failure::OutOfMemory => ("app-out-of-memory", "app-out-of-memory-body"),
    };
    let request = Request {
        app_name: entry.map_or(app_id, DesktopEntry::app_id),
        replaces_id: 0,
        app_icon: entry
            .and_then(|entry| entry.icon.as_deref())
            .unwrap_or_default(),
        summary: &L10N.tr_args(summary, [("app", app.into())]),
        body: &L10N.tr(body),
        actions: &[],
        urgency: Urgency::Normal,
        sound: None,
    };
    let told = bus.and_then(|bus| notify(&bus, &request));
    if let Err(error) = told {
        tracing::warn!("cannot tell that {app_id} failed: {error}");
    }
}

fn tell_task_stopped(bus: &Connection, unit: &str) {
    let request = Request {
        app_name: "",
        replaces_id: 0,
        app_icon: "",
        summary: &L10N.tr("task-out-of-memory"),
        body: &L10N.tr_args(
            "task-out-of-memory-body",
            [("unit", unit.to_owned().into())],
        ),
        actions: &[],
        urgency: Urgency::Normal,
        sound: None,
    };
    if let Err(error) = notify(bus, &request) {
        tracing::warn!("cannot tell that {unit} ran out of memory: {error}");
    }
}

fn killed_unit(change: &message::Message) -> Option<String> {
    let header = change.header();
    let unit = unit_of_object_path(header.path()?.as_str())?;
    let (interface, changed, _) = change
        .body()
        .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
        .ok()?;
    let result = changed.get("Result")?.downcast_ref::<&str>().ok()?;
    (UNIT_INTERFACES.contains(&interface.as_str()) && result == OOM_KILL).then_some(unit)
}

fn watch_oom_kills(bus: &Connection) -> zbus::Result<()> {
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .sender(SYSTEMD)?
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .path_namespace("/org/freedesktop/systemd1/unit")?
        .build();
    let changes = MessageIterator::for_match_rule(rule, bus, None)?;
    bus.call_method(
        Some(SYSTEMD),
        SYSTEMD_PATH,
        Some(SYSTEMD_MANAGER),
        "Subscribe",
        &(),
    )?;
    for change in changes.flatten() {
        let Some(unit) = killed_unit(&change) else {
            continue;
        };
        tracing::warn!("{unit} ran out of memory");
        let ids = unit_app_ids(&unit);
        let entries = applications(
            &data_dirs(),
            Locale::from_env().as_ref(),
            &current_desktops(),
        );
        let app_id = ids
            .iter()
            .find_map(|id| find_app(&entries, id).map(DesktopEntry::app_id))
            .or(ids.last().map(String::as_str));
        match app_id {
            Some(app_id) => tell(Ok(bus.clone()), app_id, Failure::OutOfMemory),
            None => tell_task_stopped(bus, &unit),
        }
    }
    Ok(())
}

pub fn follow_oom_kills(bus: Connection) {
    thread::spawn(move || {
        if let Err(error) = watch_oom_kills(&bus) {
            tracing::warn!("apps closed for lack of memory go untold: {error}");
        }
    });
}
