use std::fs::{self, File};
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::parent_id;
use std::path::PathBuf;

use bande_xdg::{replace_file, runtime_dir};
use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};

const FOLDER: &str = "bande/app-processes";
const SHARED: &str = "";

struct AppProcess {
    app_id: String,
    pid: u32,
    inode: u64,
    pidfd: File,
}

impl AppProcess {
    fn open(app_id: &str, pid: u32) -> io::Result<Self> {
        let raw = Pid::from_raw(pid.cast_signed()).ok_or(io::ErrorKind::InvalidInput)?;
        let pidfd = File::from(pidfd_open(raw, PidfdFlags::empty())?);
        Ok(Self {
            app_id: app_id.to_owned(),
            pid,
            inode: pidfd.metadata()?.ino(),
            pidfd,
        })
    }

    fn parse(line: &str) -> Option<Self> {
        let mut fields = line.splitn(3, ' ');
        let pid = fields.next()?.parse().ok()?;
        let inode: u64 = fields.next()?.parse().ok()?;
        Self::open(fields.next()?, pid)
            .ok()
            .filter(|process| process.inode == inode)
    }
}

pub struct AppProcesses {
    file: Option<PathBuf>,
    processes: Vec<AppProcess>,
}

impl AppProcesses {
    pub fn load() -> Self {
        let file = runtime_dir().map(|dir| dir.join(FOLDER).join(parent_id().to_string()));
        let text = file.as_ref().map(fs::read_to_string);
        let processes = match text {
            Some(Ok(text)) => text.lines().filter_map(AppProcess::parse).collect(),
            Some(Err(error)) if error.kind() != io::ErrorKind::NotFound => {
                tracing::warn!("cannot read the processes of running apps: {error}");
                Vec::new()
            }
            _ => Vec::new(),
        };
        Self { file, processes }
    }

    fn save(&self) {
        let Some(file) = &self.file else {
            return;
        };
        let lines: String = self
            .processes
            .iter()
            .map(|process| format!("{} {} {}\n", process.pid, process.inode, process.app_id))
            .collect();
        if let Err(error) = replace_file(file, lines) {
            tracing::warn!("cannot remember the processes of running apps: {error}");
        }
    }

    pub fn follow(&mut self, app_id: &str, pid: u32) {
        match self.processes.iter_mut().find(|known| known.pid == pid) {
            Some(known) if known.app_id == app_id || known.app_id == SHARED => return,
            Some(known) => known.app_id = SHARED.to_owned(),
            None => match AppProcess::open(app_id, pid) {
                Ok(process) => self.processes.push(process),
                Err(error) => {
                    tracing::debug!("cannot follow the process of {app_id}: {error}");
                    return;
                }
            },
        }
        self.save();
    }

    pub fn app_ids(&self) -> impl Iterator<Item = &str> {
        self.processes
            .iter()
            .map(|process| process.app_id.as_str())
            .filter(|app_id| *app_id != SHARED)
    }

    pub fn run(&self, app_id: &str) -> bool {
        self.app_ids().any(|known| known == app_id)
    }

    pub fn pids(&self, app_id: &str) -> Vec<u32> {
        self.of(app_id).map(|process| process.pid).collect()
    }

    fn of(&self, app_id: &str) -> impl Iterator<Item = &AppProcess> {
        self.processes
            .iter()
            .filter(move |process| process.app_id == app_id)
    }

    pub fn signal(&self, app_id: &str, signal: Signal) {
        for process in self.of(app_id) {
            if let Err(error) = pidfd_send_signal(&process.pidfd, signal) {
                tracing::warn!("cannot quit {app_id}: {error}");
            }
        }
    }

    pub fn fds(&self) -> impl Iterator<Item = BorrowedFd<'_>> {
        self.processes.iter().map(|process| process.pidfd.as_fd())
    }

    pub fn ended(&mut self, index: usize) {
        if index < self.processes.len() {
            self.processes.remove(index);
            self.save();
        }
    }
}
