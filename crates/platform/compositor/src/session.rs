use std::ffi::OsStr;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bande_protocols::server::bande_shell_v1::SessionEnd;
use rustix::io::{FdFlags, fcntl_setfd};
use rustix::process::{Pid, PidfdFlags, pidfd_open};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, Mode, PostAction};
use smithay::reexports::wayland_server::Client;

use crate::environment::{cursor_environment, desktop_environment};
use crate::state::{ClientState, Compositor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    ClipboardDaemon,
    Shell,
}

pub const CLIPBOARD_DAEMON: &str = "/usr/bin/bande-clipboard";
pub const LOCK_SCREEN: &str = "/usr/bin/bande-lock";
pub const SCREENSHOT: &str = "/usr/bin/bande-screenshot";
pub const SHELL: &str = "/usr/bin/bande-shell";
pub const GREETER: &str = "/usr/bin/bande-greeter";
pub const REVIEW: &str = "/usr/bin/bande-review";
const CONFIRM_ARGUMENT: &str = "--confirm";

pub const SESSION_CLIENT_RESTART_THROTTLE: Duration = Duration::from_secs(10);

pub(crate) struct SessionClient {
    executable: PathBuf,
    role: SessionRole,
    child: Option<Child>,
    last_restart: Option<Instant>,
}

pub const OUTPUT_VARIABLE: &str = "BANDE_OUTPUT";

pub(crate) struct LockScreen {
    executable: PathBuf,
    child: Option<Child>,
    started: Option<Instant>,
}

pub(crate) struct Summoned {
    executable: PathBuf,
    running: Option<(Child, Client)>,
}

impl Summoned {
    pub(crate) fn new(executable: &str) -> Self {
        Self {
            executable: PathBuf::from(executable),
            running: None,
        }
    }

    fn pid(&self) -> Option<u32> {
        self.running.as_ref().map(|(child, _)| child.id())
    }
}

fn session_end_keyword(ending: SessionEnd) -> &'static str {
    match ending {
        SessionEnd::Restart => "restart",
        SessionEnd::ShutDown => "shut-down",
        _ => "log-out",
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

impl Compositor {
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
        });
        Ok(())
    }

    pub fn shell_displays(&self) -> Vec<String> {
        self.state.shown_displays()
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

    pub fn set_session_dialog(&mut self, executable: &Path) {
        self.session_dialog.executable = executable.to_path_buf();
    }

    pub fn session_dialog_pid(&self) -> Option<u32> {
        self.session_dialog.pid()
    }

    fn keep_summoned(
        &mut self,
        summoned: fn(&mut Compositor) -> &mut Summoned,
        arguments: Option<&[&str]>,
        upkeep: &mut SessionUpkeep,
    ) {
        let dialog = summoned(self);
        if let Some((child, _)) = dialog.running.as_mut()
            && !matches!(child.try_wait(), Ok(None))
        {
            dialog.running = None;
        }
        let Some(arguments) = arguments else {
            return;
        };
        if let Some((_, client)) = &summoned(self).running {
            let id = client.id();
            self.state.focus_client(id);
            return;
        }
        let executable = summoned(self).executable.clone();
        let mut command = Command::new(&executable);
        command.args(arguments);
        match self.spawn_session_command(command, SessionRole::Shell) {
            Ok(running) => summoned(self).running = Some(running),
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
        let confirm = self
            .state
            .session_end_requested
            .take()
            .map(|ending| [CONFIRM_ARGUMENT, session_end_keyword(ending)]);
        self.keep_summoned(
            |compositor| &mut compositor.session_dialog,
            confirm.as_ref().map(|arguments| arguments.as_slice()),
            &mut upkeep,
        );
        self.keep_screen_reader();
        self.state.announce_displays();
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
            match self
                .spawn_session_command(Command::new(&executable), role)
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
        command.envs(desktop_environment());
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
        let executable = PathBuf::from(command.get_program());
        let mut child = command.spawn()?;
        if let Err(error) = self.wake_when_exited(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        self.state.clear_alert(&executable);
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

#[cfg(test)]
mod tests {
    use super::{CLIPBOARD_DAEMON, GREETER, LOCK_SCREEN, SCREENSHOT, SHELL};

    #[test]
    fn the_shipped_session_client_list_names_every_executable_the_compositor_starts() {
        let shipped: Vec<&str> = include_str!("../data/session-clients").lines().collect();
        assert_eq!(
            shipped,
            [CLIPBOARD_DAEMON, LOCK_SCREEN, SCREENSHOT, SHELL, GREETER]
        );
    }
}
