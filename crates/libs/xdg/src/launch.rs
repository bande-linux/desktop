use std::fmt;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::slice;
use std::sync::LazyLock;
use std::thread;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, poll};
use rustix::io::Errno;
use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open};
use unbound_inbox::{Inbox, Outbox};
use unbound_notify::Urgency;
use unbound_notify::client::{Request, notify};
use zbus::blocking::Connection;
use zbus::zvariant::Value;

use crate::L10N;
use crate::entry::{
    DesktopAction, DesktopEntry, Locale, applications, current_desktops, data_dirs,
};
use crate::identity::{app_name, find_app};
use crate::uri::file_uri;

pub const TERMINAL: &str = "unbound-term";
pub const ACTIVATION_TOKEN: &str = "XDG_ACTIVATION_TOKEN";

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
    Spawn(io::Error),
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exec(error) => write!(f, "{error}"),
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
    spawn(
        entry.app_id(),
        Some(entry),
        command,
        entry.working_directory.as_deref(),
        activation_token,
    )
    .map_err(LaunchError::Spawn)
}

pub fn spawn_detached(
    app_id: &str,
    command: &[String],
    working_directory: Option<&Path>,
    activation_token: Option<&str>,
) -> io::Result<u32> {
    spawn(app_id, None, command, working_directory, activation_token)
}

fn spawn(
    app_id: &str,
    entry: Option<&DesktopEntry>,
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
            rustix::process::setsid()
                .map(|_| ())
                .map_err(io::Error::from)
        });
    }
    let app = (!app_id.is_empty()).then(|| Supervised {
        app_id: app_id.to_owned(),
        name: entry.map(|entry| entry.name.clone()),
        icon: entry.and_then(|entry| entry.icon.clone()),
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
                watch(Watch::NotOpened(app));
            }
            Err(error)
        }
    }
}

struct Supervised {
    app_id: String,
    name: Option<String>,
    icon: Option<String>,
    started: Instant,
}

enum Watch {
    Child(Child, Option<Supervised>),
    NotOpened(Supervised),
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
            .name("app watcher".to_owned())
            .spawn(move || Watcher::new(Connection::session).run(inbox))?;
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
            let ended = match self.wait(&inbox) {
                Ok(ended) => ended,
                Err(error) => {
                    return tracing::warn!("launched apps are no longer followed: {error}");
                }
            };
            for index in ended.into_iter().rev() {
                let watched = self.watched.remove(index);
                self.finish(watched);
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

    fn bus(&mut self) -> zbus::Result<&Connection> {
        let bus = match self.bus.take() {
            Some(bus) => bus,
            None => (self.connect)()?,
        };
        Ok(self.bus.insert(bus))
    }

    fn take(&mut self, watch: Watch) {
        let (child, app) = match watch {
            Watch::NotOpened(app) => return self.tell(&app, Failure::NotOpened),
            Watch::Child(child, app) => (child, app),
        };
        if let Some(app) = &app {
            let placed = self
                .bus()
                .and_then(|bus| place_in_scope(bus, &app.app_id, child.id()));
            if let Err(error) = placed {
                tracing::debug!("{} runs outside its own scope: {error}", app.app_id);
            }
        }
        match pidfd_open(Pid::from_child(&child), PidfdFlags::empty()) {
            Ok(exited) => self.watched.push(Watched { child, exited, app }),
            Err(error) => {
                tracing::debug!("cannot follow {}: {error}", child.id());
                let mut child = child;
                let reaped = thread::Builder::new().spawn(move || child.wait());
                if let Err(error) = reaped {
                    tracing::warn!("a launched app is not followed: {error}");
                }
            }
        }
    }

    fn finish(&mut self, mut watched: Watched) {
        let status = match watched.child.wait() {
            Ok(status) => status,
            Err(error) => return tracing::debug!("cannot follow {}: {error}", watched.child.id()),
        };
        if let Some(app) = watched.app
            && let Some(failure) = failure(status, app.started.elapsed())
        {
            tracing::warn!("{} ended with {status}", app.app_id);
            self.tell(&app, failure);
        }
    }

    fn tell(&mut self, app: &Supervised, failure: Failure) {
        let (name, icon) = match &app.name {
            Some(name) => (name.clone(), app.icon.clone().unwrap_or_default()),
            None => {
                let entries = applications(
                    &data_dirs(),
                    Locale::from_env().as_ref(),
                    &current_desktops(),
                );
                (
                    app_name(&entries, &app.app_id),
                    find_app(&entries, &app.app_id)
                        .and_then(|entry| entry.icon.clone())
                        .unwrap_or_default(),
                )
            }
        };
        let (summary, body) = match failure {
            Failure::NotOpened => ("app-not-opened", "app-not-opened-body"),
            Failure::Crashed => ("app-crashed", "app-crashed-body"),
        };
        let request = Request {
            app_name: &app.app_id,
            replaces_id: 0,
            app_icon: &icon,
            summary: &L10N.tr_args(summary, [("app", name.into())]),
            body: &L10N.tr(body),
            actions: &[],
            urgency: Urgency::Normal,
            sound: None,
        };
        let told = self.bus().and_then(|bus| notify(bus, &request));
        if let Err(error) = told {
            tracing::warn!("cannot tell that {} failed: {error}", app.app_id);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    NotOpened,
    Crashed,
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

fn failure(status: ExitStatus, ran: Duration) -> Option<Failure> {
    if let Some(signal) = status.signal() {
        return CRASHES
            .iter()
            .any(|crash| crash.as_raw() == signal)
            .then_some(Failure::Crashed);
    }
    (!status.success() && ran < OPENING).then_some(Failure::NotOpened)
}

fn scope_name(app_id: &str, pid: u32) -> String {
    let mut escaped = String::with_capacity(app_id.len());
    for byte in app_id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'.') {
            escaped.push(char::from(byte));
        } else {
            escaped.push_str(&format!("\\x{byte:02x}"));
        }
    }
    format!("app-unbound-{escaped}-{pid}.scope")
}

fn place_in_scope(bus: &Connection, app_id: &str, pid: u32) -> zbus::Result<()> {
    let properties = vec![
        ("PIDs", Value::from(vec![pid])),
        ("Slice", Value::from("app.slice")),
        ("CollectMode", Value::from("inactive-or-failed")),
    ];
    bus.call_method(
        Some(SYSTEMD),
        "/org/freedesktop/systemd1",
        Some("org.freedesktop.systemd1.Manager"),
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use std::cell::Cell;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use unbound_test_bus::SessionBus;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue};

    use super::*;

    type Units = Arc<Mutex<Vec<(String, Vec<u32>, String)>>>;
    type Notes = Arc<Mutex<Vec<(String, String)>>>;

    struct Systemd(Units);

    #[zbus::interface(name = "org.freedesktop.systemd1.Manager")]
    impl Systemd {
        fn start_transient_unit(
            &self,
            name: String,
            _mode: String,
            properties: Vec<(String, OwnedValue)>,
            _aux: Vec<(String, Vec<(String, OwnedValue)>)>,
        ) -> OwnedObjectPath {
            let property = |key: &str| {
                properties
                    .iter()
                    .find(|(name, _)| name == key)
                    .map(|(_, value)| value.try_clone().unwrap())
                    .unwrap()
            };
            let pids = Vec::<u32>::try_from(property("PIDs")).unwrap();
            let slice = String::try_from(property("Slice")).unwrap();
            self.0.lock().unwrap().push((name, pids, slice));
            OwnedObjectPath::try_from("/org/freedesktop/systemd1/job/1").unwrap()
        }
    }

    struct Notifications(Notes);

    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl Notifications {
        #[allow(clippy::too_many_arguments)]
        fn notify(
            &self,
            app_name: String,
            _replaces_id: u32,
            _app_icon: String,
            summary: String,
            _body: String,
            _actions: Vec<String>,
            _hints: HashMap<String, OwnedValue>,
            _expire_timeout: i32,
        ) -> u32 {
            self.0.lock().unwrap().push((app_name, summary));
            1
        }
    }

    fn status(script: &str) -> ExitStatus {
        Command::new("sh").args(["-c", script]).status().unwrap()
    }

    #[test]
    fn only_crashes_and_failures_while_opening_are_reported() {
        assert_eq!(
            failure(status("exit 1"), Duration::ZERO),
            Some(Failure::NotOpened)
        );
        assert_eq!(
            failure(status("kill -SEGV $$"), Duration::from_secs(60)),
            Some(Failure::Crashed)
        );
        assert_eq!(
            failure(status("kill -ABRT $$"), Duration::ZERO),
            Some(Failure::Crashed)
        );
        assert_eq!(failure(status("exit 0"), Duration::ZERO), None);
        assert_eq!(failure(status("kill -TERM $$"), Duration::ZERO), None);
        assert_eq!(failure(status("kill -KILL $$"), Duration::ZERO), None);
        assert_eq!(failure(status("exit 1"), Duration::from_secs(60)), None);
    }

    #[test]
    fn apps_run_in_their_own_scope_and_a_crash_is_told() {
        let bus = SessionBus::start().unwrap();
        let units = Units::default();
        let notes = Notes::default();
        let server = bus.connect().unwrap();
        server
            .object_server()
            .at("/org/freedesktop/systemd1", Systemd(units.clone()))
            .unwrap();
        server
            .object_server()
            .at(
                "/org/freedesktop/Notifications",
                Notifications(notes.clone()),
            )
            .unwrap();
        server.request_name(SYSTEMD).unwrap();
        server
            .request_name("org.freedesktop.Notifications")
            .unwrap();

        let connections = Cell::new(0);
        let mut watcher = Watcher::new(|| {
            connections.set(connections.get() + 1);
            bus.connect()
        });
        let mut run = |app_id: &str, script: &str| {
            let child = Command::new("sh").args(["-c", script]).spawn().unwrap();
            let pid = child.id();
            let app = Supervised {
                app_id: app_id.to_owned(),
                name: None,
                icon: None,
                started: Instant::now(),
            };
            watcher.take(Watch::Child(child, Some(app)));
            pid
        };
        let calm = run("org.example.Calm", "sleep 0.2");
        let crashed = run("org.example.crash-test", "sleep 0.2; kill -SEGV $$");
        let inbox = Inbox::<Watch>::new().unwrap();
        while !watcher.watched.is_empty() {
            for index in watcher.wait(&inbox).unwrap().into_iter().rev() {
                let watched = watcher.watched.remove(index);
                watcher.finish(watched);
            }
        }
        assert_eq!(connections.get(), 1);

        assert_eq!(
            *units.lock().unwrap(),
            [
                (
                    format!("app-unbound-org.example.Calm-{calm}.scope"),
                    vec![calm],
                    "app.slice".to_owned()
                ),
                (
                    format!("app-unbound-org.example.crash\\x2dtest-{crashed}.scope"),
                    vec![crashed],
                    "app.slice".to_owned()
                ),
            ]
        );
        assert_eq!(
            *notes.lock().unwrap(),
            [(
                "org.example.crash-test".to_owned(),
                "crash-test quit unexpectedly".to_owned()
            )]
        );
    }
}
