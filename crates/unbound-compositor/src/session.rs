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
use smithay::reexports::wayland_server::Client;

use crate::state::{ClientState, Headless};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    ClipboardDaemon,
    Shell,
}

pub const CLIPBOARD_DAEMON: &str = "/usr/bin/unbound-clipboard";
pub const PANEL: &str = "/usr/bin/unbound-panel";
pub const LAUNCHER: &str = "/usr/bin/unbound-launcher";
pub const LOCK_SCREEN: &str = "/usr/bin/unbound-lock";

pub const SESSION_CLIENT_RESTART_THROTTLE: Duration = Duration::from_secs(10);

pub(crate) struct SessionClient {
    executable: PathBuf,
    role: SessionRole,
    child: Option<Child>,
    last_restart: Option<Instant>,
}

pub(crate) struct LockScreen {
    executable: PathBuf,
    child: Option<Child>,
    started: Option<Instant>,
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
        });
        Ok(())
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
        match self.spawn_session_client(&executable, SessionRole::Shell) {
            Ok(child) => self.lock_screen.child = Some(child),
            Err(error) => upkeep.failures.push((executable, error)),
        }
    }

    pub fn keep_session_clients_running(&mut self) -> SessionUpkeep {
        let mut upkeep = SessionUpkeep::default();
        self.keep_lock_screen(&mut upkeep);
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
            match self.spawn_session_client(&executable, role) {
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
        let (compositor_end, client_end) = UnixStream::pair()?;
        self.insert_session_client(compositor_end, role)?;
        let client_end = OwnedFd::from(client_end);
        let inherited = client_end.as_raw_fd();
        let mut command = Command::new(executable);
        command.env("WAYLAND_SOCKET", inherited.to_string());
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
        command.spawn()
    }
}
