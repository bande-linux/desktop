use std::ffi::OsStr;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustix::io::{FdFlags, fcntl_setfd};
use rustix::process::{Pid, PidfdFlags, pidfd_open};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, Mode, PostAction};
use smithay::reexports::wayland_server::Client;

use crate::environment::cursor_environment;
use crate::state::{ClientState, Headless};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    ClipboardDaemon,
    Shell,
}

pub const CLIPBOARD_DAEMON: &str = "/usr/bin/unbound-clipboard";
pub const DESKTOP_SURFACE: &str = "/usr/bin/unbound-desktop-surface";
pub const DOCK: &str = "/usr/bin/unbound-dock";
pub const LAUNCHER: &str = "/usr/bin/unbound-launcher";
pub const LOCK_SCREEN: &str = "/usr/bin/unbound-lock";
pub const SCREENSHOT: &str = "/usr/bin/unbound-screenshot";
pub const MENU_BAR: &str = "/usr/bin/unbound-menubar";
pub const CONTROL_CENTER: &str = "/usr/bin/unbound-controlcenter";
pub const NOTIFICATIONS: &str = "/usr/bin/unbound-notifications";
pub const GREETER: &str = "/usr/bin/unbound-greeter";
pub const REVIEW: &str = "/usr/bin/unbound-review";
pub const MONITOR: &str = "/usr/bin/unbound-monitor";
const FORCE_QUIT_ARGUMENT: &str = "--force-quit";

pub const SESSION_CLIENT_RESTART_THROTTLE: Duration = Duration::from_secs(10);

pub(crate) struct SessionClient {
    executable: PathBuf,
    role: SessionRole,
    child: Option<Child>,
    last_restart: Option<Instant>,
    display: Option<(String, bool)>,
}

pub const OUTPUT_VARIABLE: &str = "UNBOUND_OUTPUT";
pub const SECONDARY_VARIABLE: &str = "UNBOUND_SECONDARY_DISPLAY";

pub(crate) struct LockScreen {
    executable: PathBuf,
    child: Option<Child>,
    started: Option<Instant>,
}

pub(crate) struct ForceQuit {
    executable: PathBuf,
    running: Option<(Child, Client)>,
}

impl ForceQuit {
    pub(crate) fn new() -> Self {
        Self {
            executable: PathBuf::from(MONITOR),
            running: None,
        }
    }
}

impl LockScreen {
    pub(crate) fn new() -> Self {
        Self {
            executable: PathBuf::from(LOCK_SCREEN),
            child: None,
            started: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct SessionUpkeep {
    pub retry_in: Option<Duration>,
    pub failures: Vec<(PathBuf, io::Error)>,
}

impl Headless {
    pub fn set_socket_name(&mut self, name: &OsStr) {
        self.socket_name = Some(name.to_owned());
    }

    pub fn insert_session_client(
        &mut self,
        stream: UnixStream,
        role: SessionRole,
    ) -> io::Result<Client> {
        let state = ClientState {
            role: Some(role),
            ..ClientState::default()
        };
        self.display_handle().insert_client(stream, Arc::new(state))
    }

    pub fn start_session_client(&mut self, executable: &Path, role: SessionRole) -> io::Result<()> {
        let child = self.spawn_session_client(executable, role)?;
        self.session_clients.push(SessionClient {
            executable: executable.to_path_buf(),
            role,
            child: Some(child),
            last_restart: None,
            display: None,
        });
        Ok(())
    }

    pub fn start_client_on_every_display(&mut self, executable: &Path) {
        self.per_display_clients.push(executable.to_path_buf());
        self.follow_displays();
    }

    pub fn display_clients(&self, executable: &Path) -> Vec<(String, bool)> {
        self.session_clients
            .iter()
            .filter(|client| client.executable == executable)
            .filter_map(|client| client.display.clone())
            .collect()
    }

    fn follow_displays(&mut self) {
        let wanted: Vec<(PathBuf, String, bool)> = self
            .per_display_clients
            .iter()
            .flat_map(|executable| {
                self.state
                    .screens
                    .iter()
                    .enumerate()
                    .filter(|(_, screen)| screen.mirror.is_none())
                    .map(move |(index, screen)| (executable.clone(), screen.name(), index != 0))
            })
            .collect();
        self.session_clients.retain_mut(|client| {
            let Some((output, secondary)) = &client.display else {
                return true;
            };
            let keep = wanted.iter().any(|(executable, name, wanted_secondary)| {
                *executable == client.executable && name == output && wanted_secondary == secondary
            });
            if !keep && let Some(mut child) = client.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            keep
        });
        for (executable, output, secondary) in wanted {
            let known = self.session_clients.iter().any(|client| {
                client.executable == executable
                    && client
                        .display
                        .as_ref()
                        .is_some_and(|(name, _)| *name == output)
            });
            if !known {
                self.session_clients.push(SessionClient {
                    executable,
                    role: SessionRole::Shell,
                    child: None,
                    last_restart: None,
                    display: Some((output, secondary)),
                });
            }
        }
    }

    pub fn spawn_shell_client(&mut self, executable: &Path) -> io::Result<Child> {
        self.spawn_session_client(executable, SessionRole::Shell)
    }

    pub fn session_client_pid(&self, executable: &Path) -> Option<u32> {
        self.session_clients
            .iter()
            .find(|client| client.executable == executable)
            .and_then(|client| client.child.as_ref())
            .map(Child::id)
    }

    pub fn set_lock_screen(&mut self, executable: &Path) {
        self.lock_screen.executable = executable.to_path_buf();
    }

    pub fn request_lock_screen(&mut self) {
        self.state.lock_screen_requested = !self.state.is_locked();
    }

    pub fn report_lock_states(&mut self, states: std::sync::mpsc::Sender<bool>) {
        self.state.lock_states = Some(states);
    }

    pub fn set_force_quit(&mut self, executable: &Path) {
        self.force_quit.executable = executable.to_path_buf();
    }

    pub fn force_quit_pid(&self) -> Option<u32> {
        self.force_quit
            .running
            .as_ref()
            .map(|(child, _)| child.id())
    }

    fn keep_force_quit(&mut self, upkeep: &mut SessionUpkeep) {
        if let Some((child, _)) = self.force_quit.running.as_mut()
            && !matches!(child.try_wait(), Ok(None))
        {
            self.force_quit.running = None;
        }
        if !std::mem::take(&mut self.state.force_quit_requested) {
            return;
        }
        if let Some((_, client)) = &self.force_quit.running {
            self.state.focus_client(client.id());
            return;
        }
        let executable = self.force_quit.executable.clone();
        let mut command = Command::new(&executable);
        command.arg(FORCE_QUIT_ARGUMENT);
        match self.spawn_session_command(command, SessionRole::Shell) {
            Ok(running) => self.force_quit.running = Some(running),
            Err(error) => upkeep.failures.push((executable, error)),
        }
    }

    pub fn lock_screen_pid(&self) -> Option<u32> {
        self.lock_screen.child.as_ref().map(Child::id)
    }

    fn keep_lock_screen(&mut self, upkeep: &mut SessionUpkeep) {
        if let Some(child) = self.lock_screen.child.as_mut() {
            match child.try_wait() {
                Ok(None) => {
                    self.state.lock_screen_requested = false;
                    return;
                }
                Ok(Some(_)) => self.lock_screen.child = None,
                Err(error) => {
                    upkeep
                        .failures
                        .push((self.lock_screen.executable.clone(), error));
                    return;
                }
            }
        }
        let relock = self.state.lock_needs_locker();
        if !self.state.lock_screen_requested && !relock {
            return;
        }
        if relock
            && let Some(since) = self.lock_screen.started.map(|started| started.elapsed())
            && since < SESSION_CLIENT_RESTART_THROTTLE
        {
            let wait = SESSION_CLIENT_RESTART_THROTTLE - since;
            upkeep.retry_in = Some(upkeep.retry_in.map_or(wait, |retry| retry.min(wait)));
            return;
        }
        self.state.lock_screen_requested = false;
        self.lock_screen.started = Some(Instant::now());
        let executable = self.lock_screen.executable.clone();
        let mut command = Command::new(&executable);
        command.env(OUTPUT_VARIABLE, self.state.main_screen().name());
        match self
            .spawn_session_command(command, SessionRole::Shell)
            .map(|(child, _)| child)
        {
            Ok(child) => self.lock_screen.child = Some(child),

            Err(error) => upkeep.failures.push((executable, error)),
        }
    }

    pub fn keep_session_clients_running(&mut self) -> SessionUpkeep {
        let mut upkeep = SessionUpkeep::default();
        self.keep_lock_screen(&mut upkeep);
        self.keep_force_quit(&mut upkeep);
        self.follow_displays();
        for index in 0..self.session_clients.len() {
            let client = &mut self.session_clients[index];
            if let Some(child) = client.child.as_mut() {
                match child.try_wait() {
                    Ok(None) => continue,
                    Ok(Some(_)) => client.child = None,
                    Err(error) => {
                        upkeep.failures.push((client.executable.clone(), error));
                        continue;
                    }
                }
            }
            if let Some(since) = client.last_restart.map(|last| last.elapsed())
                && since < SESSION_CLIENT_RESTART_THROTTLE
            {
                let wait = SESSION_CLIENT_RESTART_THROTTLE - since;
                upkeep.retry_in = Some(upkeep.retry_in.map_or(wait, |retry| retry.min(wait)));
                continue;
            }
            client.last_restart = Some(Instant::now());
            let (executable, role) = (client.executable.clone(), client.role);
            let mut command = Command::new(&executable);
            if let Some((output, secondary)) = &client.display {
                command.env(OUTPUT_VARIABLE, output);
                if *secondary {
                    command.env(SECONDARY_VARIABLE, "1");
                }
            }
            match self
                .spawn_session_command(command, role)
                .map(|(child, _)| child)
            {
                Ok(child) => self.session_clients[index].child = Some(child),
                Err(error) => {
                    upkeep.failures.push((executable, error));
                    let wait = SESSION_CLIENT_RESTART_THROTTLE;
                    upkeep.retry_in = Some(upkeep.retry_in.map_or(wait, |retry| retry.min(wait)));
                }
            }
        }
        upkeep
    }

    fn spawn_session_client(&mut self, executable: &Path, role: SessionRole) -> io::Result<Child> {
        self.spawn_session_command(Command::new(executable), role)
            .map(|(child, _)| child)
    }

    fn spawn_session_command(
        &mut self,
        mut command: Command,
        role: SessionRole,
    ) -> io::Result<(Child, Client)> {
        let (compositor_end, client_end) = UnixStream::pair()?;
        let client = self.insert_session_client(compositor_end, role)?;
        let client_end = OwnedFd::from(client_end);
        let inherited = client_end.as_raw_fd();
        command.env("WAYLAND_SOCKET", inherited.to_string());
        command.envs(cursor_environment(self.cursor_theme()));
        match (&self.socket_name, role) {
            (Some(name), SessionRole::Shell) => command.env("WAYLAND_DISPLAY", name),
            _ => command.env_remove("WAYLAND_DISPLAY"),
        };
        unsafe {
            command.pre_exec(move || {
                fcntl_setfd(BorrowedFd::borrow_raw(inherited), FdFlags::empty())
                    .map_err(io::Error::from)
            });
        }
        let mut child = command.spawn()?;
        if let Err(error) = self.wake_when_exited(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok((child, client))
    }

    fn wake_when_exited(&self, child: &Child) -> io::Result<()> {
        let exited = pidfd_open(Pid::from_child(child), PidfdFlags::empty())?;
        self.state
            .blockers
            .insert_source(
                Generic::new(exited, Interest::READ, Mode::Level),
                |_, _, _| Ok(PostAction::Remove),
            )
            .map(drop)
            .map_err(|error| io::Error::other(error.error))
    }
}
