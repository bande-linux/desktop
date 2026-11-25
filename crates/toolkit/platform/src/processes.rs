use std::collections::HashSet;
use std::os::fd::OwnedFd;
use std::sync::Arc;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::process::{Pid, PidfdFlags, pidfd_open};

#[derive(Debug, Clone)]
pub struct AppProcess {
    pub pid: u32,
    pub handle: Arc<OwnedFd>,
}

impl AppProcess {
    fn open(pid: u32) -> Option<Self> {
        let raw = Pid::from_raw(pid.cast_signed())?;
        let handle = pidfd_open(raw, PidfdFlags::empty())
            .inspect_err(|error| tracing::debug!("cannot follow process {pid}: {error}"))
            .ok()?;
        Some(Self {
            pid,
            handle: Arc::new(handle),
        })
    }

    fn ended(&self) -> bool {
        let mut fds = [PollFd::new(&*self.handle, PollFlags::IN)];
        poll(&mut fds, Some(&Timespec::default())).is_ok_and(|ready| ready > 0)
    }
}

struct Known {
    app_id: String,
    process: AppProcess,
    confirmed: bool,
    doubted: bool,
}

#[derive(Clone)]
pub(crate) struct Announced {
    app_id: String,
    process: AppProcess,
}

impl Announced {
    fn is(&self, known: &Known) -> bool {
        known.app_id == self.app_id && Arc::ptr_eq(&known.process.handle, &self.process.handle)
    }
}

#[derive(Default)]
pub(crate) struct Processes {
    known: Vec<Known>,
    ended: HashSet<u32>,
}

impl Processes {
    fn drop_ended(&mut self) {
        let ended = &mut self.ended;
        self.known.retain(|known| {
            let runs = !known.process.ended();
            if !runs {
                ended.insert(known.process.pid);
            }
            runs
        });
    }

    pub(crate) fn announce(&mut self, app_id: &str, pid: u32) -> Option<Announced> {
        self.drop_ended();
        let known = self
            .known
            .iter()
            .any(|known| known.app_id == app_id && known.process.pid == pid);
        if known || self.ended.contains(&pid) {
            return None;
        }
        let process = AppProcess::open(pid)?;
        self.known.push(Known {
            app_id: app_id.to_owned(),
            process: process.clone(),
            confirmed: false,
            doubted: false,
        });
        Some(Announced {
            app_id: app_id.to_owned(),
            process,
        })
    }

    /// The compositor answered after the handle was opened, and no window of the app closed
    /// meanwhile: the announced connection, and so its process, was still there.
    pub(crate) fn confirm(&mut self, announced: &Announced) {
        self.drop_ended();
        if let Some(known) = self.known.iter_mut().find(|known| announced.is(known)) {
            known.confirmed = !known.doubted;
        }
    }

    pub(crate) fn window_closed(&mut self, closed: &str, open: impl Fn(&str) -> bool) {
        self.drop_ended();
        self.known.retain(|known| open(&known.app_id));
        for known in &mut self.known {
            known.doubted |= known.app_id == closed;
        }
    }

    pub(crate) fn latest_pid(&self, app_id: &str) -> Option<u32> {
        self.known
            .iter()
            .rfind(|known| known.app_id == app_id && !known.process.ended())
            .map(|known| known.process.pid)
    }

    pub(crate) fn of(&self, app_id: &str) -> Vec<AppProcess> {
        self.known
            .iter()
            .filter(|known| known.app_id == app_id && known.confirmed && !known.process.ended())
            .map(|known| known.process.clone())
            .collect()
    }
}
