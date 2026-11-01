use std::fs;
use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;

use bande_xdg::unit_app_ids;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;
use rustix::net::sockopt::socket_peercred;
use rustix::process::{PidfdFlags, Signal, pidfd_open, pidfd_send_signal};

const PROC: &str = "/proc";
const CGROUPS: &str = "/sys/fs/cgroup";
const KILL_FILE: &str = "cgroup.kill";

pub(crate) struct Process {
    pidfd: OwnedFd,
    pid: i32,
}

fn peer_pidfd(stream: &UnixStream) -> Option<OwnedFd> {
    let mut fd: libc::c_int = -1;
    let mut length = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    let found = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERPIDFD,
            (&raw mut fd).cast(),
            &raw mut length,
        )
    };
    (found == 0 && fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
}

impl Process {
    pub(crate) fn of_peer(stream: &UnixStream) -> Option<Self> {
        let pid = socket_peercred(stream).ok()?.pid;
        let pidfd = peer_pidfd(stream).or_else(|| pidfd_open(pid, PidfdFlags::empty()).ok())?;
        Some(Self {
            pidfd,
            pid: pid.as_raw_nonzero().get(),
        })
    }

    pub(crate) fn pid(&self) -> i32 {
        self.pid
    }

    fn has_exited(&self) -> bool {
        let mut fds = [PollFd::new(&self.pidfd, PollFlags::IN)];
        let now = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        poll(&mut fds, Some(&now)).is_ok_and(|ready| ready > 0)
    }

    pub(crate) fn name(&self) -> Option<String> {
        let name = fs::read_to_string(Path::new(PROC).join(self.pid.to_string()).join("comm"));
        let name = name.ok()?.trim_end().to_owned();
        (!self.has_exited() && !name.is_empty()).then_some(name)
    }

    pub(crate) fn force_quit(&self, app_ids: &[String]) -> io::Result<()> {
        let theirs = fs::read_to_string(Path::new(PROC).join(self.pid.to_string()).join("cgroup"))
            .unwrap_or_default();
        let own = fs::read_to_string(Path::new(PROC).join("self/cgroup")).unwrap_or_default();
        if self.has_exited() {
            return Ok(());
        }
        let scope = app_scope(&theirs, &own, app_ids);
        match pidfd_send_signal(&self.pidfd, Signal::KILL) {
            Ok(()) | Err(Errno::SRCH) => {}
            Err(error) => return Err(error.into()),
        }
        let killed = scope.map(|scope| kill_scope(Path::new(CGROUPS), scope));
        self.release_memory();
        killed.unwrap_or(Ok(()))
    }

    fn release_memory(&self) {
        let Ok(pidfd) = self.pidfd.as_fd().try_clone_to_owned() else {
            return;
        };
        let released = std::thread::Builder::new()
            .name("release-memory".to_owned())
            .spawn(move || {
                let result =
                    unsafe { libc::syscall(libc::SYS_process_mrelease, pidfd.as_raw_fd(), 0) };
                if result != 0 {
                    tracing::debug!(
                        "cannot release the memory of a killed app: {}",
                        io::Error::last_os_error()
                    );
                }
            });
        if let Err(error) = released {
            tracing::debug!("cannot release the memory of a killed app: {error}");
        }
    }
}

fn cgroup_path(cgroup: &str) -> Option<&str> {
    cgroup
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .filter(|path| path.len() > 1)
}

pub(crate) fn app_scope<'a>(theirs: &'a str, own: &str, app_ids: &[String]) -> Option<&'a str> {
    let path = cgroup_path(theirs)?;
    let own = cgroup_path(own).unwrap_or("/");
    if own == path || own.starts_with(&format!("{path}/")) {
        return None;
    }
    let unit = path.rsplit('/').next()?;
    unit_app_ids(unit)
        .iter()
        .any(|named| app_ids.iter().any(|id| id.eq_ignore_ascii_case(named)))
        .then_some(path)
}

pub(crate) fn kill_scope(root: &Path, scope: &str) -> io::Result<()> {
    fs::write(
        root.join(scope.trim_start_matches('/')).join(KILL_FILE),
        "1",
    )
}
