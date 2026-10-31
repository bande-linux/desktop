use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
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
