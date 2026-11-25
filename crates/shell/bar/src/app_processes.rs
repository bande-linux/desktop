use std::cell::RefCell;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::rc::Rc;

use rustix::process::{Pid, PidfdFlags, pidfd_open};

thread_local! {
    static KNOWN: RefCell<Vec<AppProcess>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone)]
struct AppProcess {
    app_id: Option<String>,
    pid: u32,
    pidfd: Rc<OwnedFd>,
}

impl AppProcess {
    fn open(app_id: &str, pid: u32) -> io::Result<Self> {
        let raw = Pid::from_raw(pid.cast_signed()).ok_or(io::ErrorKind::InvalidInput)?;
        Ok(Self {
            app_id: Some(app_id.to_owned()),
            pid,
            pidfd: Rc::new(pidfd_open(raw, PidfdFlags::empty())?),
        })
    }
}

pub struct AppProcesses(Vec<AppProcess>);

impl AppProcesses {
    pub fn known() -> Self {
        Self(KNOWN.with_borrow(Clone::clone))
    }

    fn share(&self) {
        KNOWN.set(self.0.clone());
    }

    pub fn follow(&mut self, app_id: &str, pid: u32) {
        match self.0.iter_mut().find(|known| known.pid == pid) {
            Some(known) if known.app_id.as_deref().is_none_or(|known| known == app_id) => return,
            Some(known) => known.app_id = None,
            None => match AppProcess::open(app_id, pid) {
                Ok(process) => self.0.push(process),
                Err(error) => {
                    tracing::debug!("cannot follow the process of {app_id}: {error}");
                    return;
                }
            },
        }
        self.share();
    }

    pub fn app_ids(&self) -> impl Iterator<Item = &str> {
        self.0
            .iter()
            .filter_map(|process| process.app_id.as_deref())
    }

    pub fn run(&self, app_id: &str) -> bool {
        self.app_ids().any(|known| known == app_id)
    }

    pub fn is_of(&self, pid: u32, app_id: &str) -> bool {
        self.0
            .iter()
            .any(|process| process.pid == pid && process.app_id.as_deref() == Some(app_id))
    }

    pub fn fds(&self) -> impl Iterator<Item = BorrowedFd<'_>> {
        self.0.iter().map(|process| process.pidfd.as_fd())
    }

    pub fn ended(&mut self, index: usize) {
        if index < self.0.len() {
            self.0.remove(index);
            self.share();
        }
    }
}
