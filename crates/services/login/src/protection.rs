use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::{fs, io, ptr};

use bande_bus::systemd::{ServiceManagerProxyBlocking, main_pid, memory_min, set_memory_min};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::process::{Pid, PidfdFlags, pidfd_open};
use zbus::blocking::{Connection, connection};

use crate::checked;

const MEMINFO: &str = "/proc/meminfo";
const USERS_SLICE: &str = "user.slice";
const USER_SLICES: &str = "user-*.slice";
const MOST: u64 = 512 << 20;
const SHARE: u64 = 10;
const RUNTIME_PROPERTIES: &str = "/run/systemd/system.control";
const MEMORY_MIN_DROP_IN: &str = "50-MemoryMin.conf";
const ANSWER_PATIENCE: Duration = Duration::from_secs(2);
const LOCKABLE: u64 = 256 << 20;

struct Lockable {
    manager: Pid,
    running: OwnedFd,
    before: libc::rlimit,
}

impl Lockable {
    fn undo(&self) -> io::Result<()> {
        let mut ended = [PollFd::new(&self.running, PollFlags::IN)];
        if poll(&mut ended, Some(&Timespec::default()))? == 0 {
            lock_limit(self.manager, Some(&self.before))?;
        }
        Ok(())
    }
}

/// Memory kept for one person's desktop on every level above their session while it runs, so the
/// kernel does not take the desktop's pages when apps want more memory than there is. What the
/// desktop leaves unused of it, other processes of the same level may use.
pub struct Protection {
    ended: Option<Sender<()>>,
    kept: Option<JoinHandle<()>>,
    lockable: Option<Lockable>,
}

struct Kept {
    system: Connection,
    uid: u32,
    bytes: u64,
    held: Vec<String>,
}

fn total_memory() -> Option<u64> {
    let meminfo = fs::read_to_string(MEMINFO).ok()?;
    let line = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?;
    let kilobytes: u64 = line.split_whitespace().next()?.parse().ok()?;
    Some(kilobytes * 1024)
}

fn lock_limit(process: Pid, new: Option<&libc::rlimit>) -> io::Result<libc::rlimit> {
    let mut old = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    let new = new.map_or(ptr::null(), ptr::from_ref);
    // SAFETY: new is null or points to a limit, and old to one, both outliving the call.
    checked(unsafe {
        libc::prlimit(
            process.as_raw_nonzero().get(),
            libc::RLIMIT_MEMLOCK,
            new,
            &raw mut old,
        )
    })?;
    Ok(old)
}

fn drop_in(unit: &str) -> PathBuf {
    Path::new(RUNTIME_PROPERTIES).join(format!("{unit}.d/{MEMORY_MIN_DROP_IN}"))
}

fn slice_of(uid: u32) -> String {
    format!("user-{uid}.slice")
}

fn let_lock_memory(system: &Connection, user_manager: &str) -> zbus::Result<Option<Lockable>> {
    let manager =
        main_pid(system, user_manager)?.and_then(|pid| Pid::from_raw(pid.try_into().ok()?));
    let Some(manager) = manager else {
        return Ok(None);
    };
    let running = pidfd_open(manager, PidfdFlags::empty()).map_err(io::Error::from)?;
    let before = lock_limit(manager, None)?;
    if before.rlim_max >= LOCKABLE {
        return Ok(None);
    }
    let raised = libc::rlimit {
        rlim_max: LOCKABLE,
        ..before
    };
    lock_limit(manager, Some(&raised))?;
    Ok(Some(Lockable {
        manager,
        running,
        before,
    }))
}

fn user_manager_of(uid: u32) -> String {
    format!("user@{uid}.service")
}

impl Protection {
    pub fn of(uid: u32) -> Option<Self> {
        let system = connection::Builder::system()
            .and_then(|builder| builder.method_timeout(ANSWER_PATIENCE).build())
            .inspect_err(|error| tracing::warn!("no memory is kept for the desktop: {error}"))
            .ok()?;
        let lockable = let_lock_memory(&system, &user_manager_of(uid))
            .inspect_err(|error| {
                tracing::warn!("the desktop cannot lock its code in memory: {error}");
            })
            .ok()
            .flatten();
        let (ended, until) = mpsc::channel();
        let kept = thread::Builder::new()
            .name("kept-memory".to_owned())
            .spawn(move || {
                if let Some(kept) = Kept::hold_all(system, uid) {
                    let _ = until.recv();
                    kept.release_all();
                }
            })
            .inspect_err(|error| tracing::warn!("no memory is kept for the desktop: {error}"))
            .ok();
        Some(Self {
            ended: Some(ended),
            kept,
            lockable,
        })
    }
}

impl Kept {
    fn hold_all(system: Connection, uid: u32) -> Option<Self> {
        let bytes = total_memory().map(|total| (total / SHARE).min(MOST))?;
        let mut kept = Self {
            system,
            uid,
            bytes,
            held: Vec::new(),
        };
        for unit in [user_manager_of(uid), slice_of(uid), USERS_SLICE.to_owned()] {
            match kept.hold(&unit) {
                Ok(true) => kept.held.push(unit),
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!("no memory is kept for the desktop in {unit}: {error}");
                }
            }
        }
        Some(kept)
    }

    fn hold(&self, unit: &str) -> zbus::Result<bool> {
        let current = memory_min(&self.system, unit)?;
        if drop_in(unit).exists() {
            return Ok(current == self.bytes);
        }
        if current == 0 {
            set_memory_min(&self.system, unit, self.bytes)?;
        }
        Ok(current == 0)
    }

    fn others_hold(&self) -> zbus::Result<bool> {
        let own = slice_of(self.uid);
        let units = ServiceManagerProxyBlocking::new(&self.system)?
            .list_units_by_patterns(&["active"], &[USER_SLICES])?;
        for unit in units.iter().map(|unit| unit.0.as_str()) {
            if unit != own && memory_min(&self.system, unit)? == self.bytes {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn release_all(&self) {
        for unit in &self.held {
            if let Err(error) = self.forget(unit) {
                tracing::warn!("the memory kept for the desktop stays kept in {unit}: {error}");
            }
        }
    }

    fn forget(&self, unit: &str) -> zbus::Result<()> {
        if unit == USERS_SLICE && self.others_hold()? {
            return Ok(());
        }
        set_memory_min(&self.system, unit, 0)?;
        let drop_in = drop_in(unit);
        fs::remove_file(&drop_in)?;
        if let Some(folder) = drop_in.parent() {
            let _ = fs::remove_dir(folder);
        }
        Ok(())
    }
}

impl Drop for Protection {
    fn drop(&mut self) {
        drop(self.ended.take());
        if let Some(kept) = self.kept.take()
            && kept.join().is_err()
        {
            tracing::warn!("the memory kept for the desktop stays kept");
        }
        if let Some(lockable) = &self.lockable
            && let Err(error) = lockable.undo()
        {
            tracing::warn!("the person may still lock more memory: {error}");
        }
    }
}
