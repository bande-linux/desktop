use std::fs;

use bande_bus::systemd::{ServiceManagerProxyBlocking, memory_min, set_memory_min};
use zbus::blocking::Connection;

const MEMINFO: &str = "/proc/meminfo";
const USERS_SLICE: &str = "user.slice";
const USER_SLICES: &str = "user-*.slice";
const MOST: u64 = 512 << 20;
const SHARE: u64 = 10;

/// Memory kept for one person's desktop on every level above their session while it runs, so the
/// kernel does not take the desktop's pages when apps want more memory than there is.
pub struct Protection {
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

fn slice_of(uid: u32) -> String {
    format!("user-{uid}.slice")
}

impl Protection {
    pub fn of(uid: u32) -> Option<Self> {
        let bytes = total_memory().map(|total| (total / SHARE).min(MOST))?;
        let system = bande_bus::system()
            .inspect_err(|error| tracing::warn!("no memory is kept for the desktop: {error}"))
            .ok()?;
        let mut protection = Self {
            system,
            uid,
            bytes,
            held: Vec::new(),
        };
        for unit in [
            USERS_SLICE.to_owned(),
            slice_of(uid),
            format!("user@{uid}.service"),
        ] {
            match protection.hold(&unit) {
                Ok(true) => protection.held.push(unit),
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!("no memory is kept for the desktop in {unit}: {error}")
                }
            }
        }
        Some(protection)
    }

    fn hold(&self, unit: &str) -> zbus::Result<bool> {
        let current = memory_min(&self.system, unit)?;
        if current == 0 {
            set_memory_min(&self.system, unit, self.bytes)?;
        }
        Ok(current == 0 || current == self.bytes)
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

    fn release(&self, unit: &str) -> zbus::Result<()> {
        if unit == USERS_SLICE && self.others_hold()? {
            return Ok(());
        }
        set_memory_min(&self.system, unit, 0)
    }
}

impl Drop for Protection {
    fn drop(&mut self) {
        for unit in self.held.iter().rev() {
            if let Err(error) = self.release(unit) {
                tracing::warn!("the memory kept for the desktop stays kept in {unit}: {error}");
            }
        }
    }
}
