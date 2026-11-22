use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

const READ: u32 = 0o4;
const WRITE: u32 = 0o2;
const EXECUTE: u32 = 0o1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Owner,
    Group,
    Others,
}

impl Class {
    pub const ALL: [Self; 3] = [Self::Owner, Self::Group, Self::Others];

    fn shift(self) -> u32 {
        match self {
            Self::Owner => 6,
            Self::Group => 3,
            Self::Others => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    ReadWrite,
    ReadOnly,
    NoAccess,
}

impl Access {
    pub const ALL: [Self; 3] = [Self::ReadWrite, Self::ReadOnly, Self::NoAccess];
}

fn bits(mode: u32, class: Class) -> u32 {
    (mode >> class.shift()) & 0o7
}

pub fn access(mode: u32, class: Class) -> Access {
    let bits = bits(mode, class);
    match (bits & READ != 0, bits & WRITE != 0) {
        (true, true) => Access::ReadWrite,
        (true, false) => Access::ReadOnly,
        (false, _) => Access::NoAccess,
    }
}

pub fn with_access(mode: u32, class: Class, access: Access, folder: bool) -> u32 {
    let old = bits(mode, class);
    let new = match (access, folder) {
        (Access::ReadWrite, true) => READ | WRITE | EXECUTE,
        (Access::ReadOnly, true) => READ | EXECUTE,
        (Access::ReadWrite, false) => READ | WRITE | (old & EXECUTE),
        (Access::ReadOnly, false) => READ | (old & EXECUTE),
        (Access::NoAccess, _) => 0,
    };
    (mode & !(0o7 << class.shift())) | (new << class.shift())
}

pub fn runnable(mode: u32) -> bool {
    mode & 0o111 != 0
}

pub fn with_runnable(mode: u32, on: bool) -> u32 {
    let readers = Class::ALL
        .into_iter()
        .filter(|class| bits(mode, *class) & READ != 0)
        .fold(0, |bits, class| bits | (EXECUTE << class.shift()));
    match on {
        true => mode | readers,
        false => mode & !0o111,
    }
}

pub fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o7777))
}

pub fn set_group(path: &Path, group: u32) -> io::Result<()> {
    std::os::unix::fs::lchown(path, None, Some(group))
}

/// The groups this user belongs to, by id and name, the primary group first.
pub fn own_groups() -> Vec<(u32, String)> {
    let names: HashMap<u32, String> = fs::read_to_string("/etc/group")
        .inspect_err(|error| tracing::debug!("cannot read the groups: {error}"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let id = fields.nth(1)?.parse().ok()?;
            Some((id, name.to_owned()))
        })
        .collect();
    let mut ids = vec![rustix::process::getgid().as_raw()];
    match rustix::process::getgroups() {
        Ok(groups) => ids.extend(groups.into_iter().map(|group| group.as_raw())),
        Err(error) => tracing::warn!("cannot find this user's groups: {error}"),
    }
    let mut seen = HashSet::new();
    ids.retain(|id| seen.insert(*id));
    ids.into_iter()
        .map(|id| {
            (
                id,
                names.get(&id).cloned().unwrap_or_else(|| id.to_string()),
            )
        })
        .collect()
}

/// Gives everything inside `folder`, on the same file system and not through links, the access
/// `mode` gives each class and, when given, the `group`. Carries on past failures and returns the
/// first.
pub fn apply_to_enclosed(folder: &Path, mode: u32, group: Option<u32>) -> io::Result<()> {
    let device = fs::symlink_metadata(folder)?.dev();
    let mut first = None;
    let mut waiting = vec![folder.to_path_buf()];
    while let Some(next) = waiting.pop() {
        let entries = match fs::read_dir(&next) {
            Ok(entries) => entries,
            Err(error) => {
                first.get_or_insert(error);
                continue;
            }
        };
        for entry in entries {
            let changed = entry.and_then(|entry| {
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path)?;
                if metadata.is_symlink() || metadata.dev() != device {
                    return Ok(());
                }
                let is_folder = metadata.is_dir();
                let new = Class::ALL.into_iter().fold(metadata.mode(), |old, class| {
                    with_access(old, class, access(mode, class), is_folder)
                });
                if let Some(group) = group.filter(|group| *group != metadata.gid()) {
                    set_group(&path, group)?;
                }
                if new != metadata.mode() {
                    set_mode(&path, new)?;
                }
                if is_folder {
                    waiting.push(path);
                }
                Ok(())
            });
            if let Err(error) = changed {
                first.get_or_insert(error);
            }
        }
    }
    first.map_or(Ok(()), Err)
}
