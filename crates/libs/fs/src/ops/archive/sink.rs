use std::ffi::OsStr;
use std::fs::{File, Permissions};
use std::io::{self, Write};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use rustix::fs::{
    AtFlags, FileType, Mode, OFlags, ResolveFlags, fchmod, linkat, mkdirat, openat, openat2,
    readlinkat, statat, symlinkat, unlinkat,
};
use rustix::io::Errno;

use super::Refusal;

const KEPT_MODE: u32 = 0o777;
const OWNER_MODE: u32 = 0o700;
const BOMB_RATIO: u64 = 1000;
const BOMB_FLOOR: u64 = 64 << 20;

pub(super) fn kept(mode: u32) -> Permissions {
    Permissions::from_mode(mode & KEPT_MODE)
}

pub(super) fn entry_path(name: &Path) -> io::Result<Option<PathBuf>> {
    let mut clean = PathBuf::new();
    for part in name.components() {
        match part {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            _ => return Err(Refusal::Outside.into()),
        }
    }
    Ok(Some(clean).filter(|clean| !clean.as_os_str().is_empty()))
}

fn stays_inside(link: &Path, target: &Path) -> bool {
    let mut depth = link.components().count().saturating_sub(1);
    target.components().all(|part| match part {
        Component::Normal(_) => {
            depth += 1;
            true
        }
        Component::CurDir => true,
        Component::ParentDir => depth.checked_sub(1).map(|up| depth = up).is_some(),
        Component::RootDir | Component::Prefix(_) => false,
    })
}

pub(super) struct Budget {
    allowed: u64,
    refusal: Refusal,
    written: u64,
}

impl Budget {
    pub(super) fn new(packed: u64, free: Option<u64>) -> Self {
        let ratio = packed.saturating_mul(BOMB_RATIO).max(BOMB_FLOOR);
        let free = free.unwrap_or(u64::MAX);
        Self {
            allowed: ratio.min(free),
            refusal: match ratio <= free {
                true => Refusal::Bomb,
                false => Refusal::TooLarge,
            },
            written: 0,
        }
    }

    pub(super) fn admits(&self, declared: u64) -> io::Result<()> {
        match declared > self.allowed {
            true => Err(self.refusal.into()),
            false => Ok(()),
        }
    }

    fn take(&mut self, bytes: u64) -> io::Result<()> {
        self.written = self.written.saturating_add(bytes);
        self.admits(self.written)
    }
}

pub(super) struct Metered<'a> {
    file: &'a mut File,
    budget: &'a mut Budget,
}

impl Write for Metered<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.budget.take(buffer.len() as u64)?;
        self.file.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

pub(super) struct Sink {
    root: OwnedFd,
    budget: Budget,
    folders: Vec<(PathBuf, u32)>,
    links: Vec<PathBuf>,
}

impl Sink {
    pub(super) fn open(folder: &Path, budget: Budget) -> io::Result<Self> {
        let root = openat(
            rustix::fs::CWD,
            folder,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        Ok(Self {
            root,
            budget,
            folders: Vec::new(),
            links: Vec::new(),
        })
    }

    pub(super) fn budget(&self) -> &Budget {
        &self.budget
    }

    pub(super) fn metered<'a>(&'a mut self, file: &'a mut File) -> Metered<'a> {
        Metered {
            file,
            budget: &mut self.budget,
        }
    }

    fn walk(&self, folders: &Path, create: bool) -> io::Result<OwnedFd> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut dir = openat(&self.root, ".", flags, Mode::empty())?;
        for part in folders.components() {
            let name = part.as_os_str();
            if create {
                match mkdirat(&dir, name, Mode::from_raw_mode(0o777)) {
                    Ok(()) | Err(Errno::EXIST) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            dir = match openat(&dir, name, flags, Mode::empty()) {
                Ok(next) => next,
                Err(error) => {
                    let link = statat(&dir, name, AtFlags::SYMLINK_NOFOLLOW).is_ok_and(|stat| {
                        FileType::from_raw_mode(stat.st_mode) == FileType::Symlink
                    });
                    return Err(match link {
                        true => Refusal::Outside.into(),
                        false => error.into(),
                    });
                }
            };
        }
        Ok(dir)
    }

    fn split<'a>(&self, path: &'a Path, create: bool) -> io::Result<(OwnedFd, &'a OsStr)> {
        let name = path.file_name().ok_or(Refusal::Outside)?;
        let parent = path.parent().unwrap_or(Path::new(""));
        Ok((self.walk(parent, create)?, name))
    }

    fn make<T>(
        &self,
        path: &Path,
        mut create: impl FnMut(&OwnedFd, &OsStr) -> rustix::io::Result<T>,
    ) -> io::Result<T> {
        let (dir, name) = self.split(path, true)?;
        match create(&dir, name) {
            Err(Errno::EXIST) => {
                unlinkat(&dir, name, AtFlags::empty())?;
                Ok(create(&dir, name)?)
            }
            made => Ok(made?),
        }
    }

    pub(super) fn folder(&mut self, path: &Path, mode: Option<u32>) -> io::Result<()> {
        self.walk(path, true)?;
        if let Some(mode) = mode {
            self.folders.push((path.to_path_buf(), mode));
        }
        Ok(())
    }

    pub(super) fn file(&mut self, path: &Path) -> io::Result<File> {
        let flags = OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW;
        self.make(path, |dir, name| {
            openat(
                dir,
                name,
                flags | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o666),
            )
        })
        .map(File::from)
    }

    pub(super) fn symlink(&mut self, path: &Path, target: &Path) -> io::Result<()> {
        self.make(path, |dir, name| symlinkat(target, dir, name))?;
        self.links.push(path.to_path_buf());
        Ok(())
    }

    pub(super) fn hard_link(&mut self, path: &Path, target: &Path) -> io::Result<()> {
        let (from, original) = self.split(target, false)?;
        self.make(path, |dir, name| {
            linkat(&from, original, dir, name, AtFlags::empty())
        })?;
        let (dir, name) = self.split(path, false)?;
        let stat = statat(&dir, name, AtFlags::SYMLINK_NOFOLLOW)?;
        if FileType::from_raw_mode(stat.st_mode) == FileType::Symlink {
            self.links.push(path.to_path_buf());
        }
        Ok(())
    }

    fn check_link(&self, link: &Path) -> io::Result<()> {
        let resolved = openat2(
            &self.root,
            link,
            OFlags::PATH | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH,
        );
        match resolved {
            Ok(_) | Err(Errno::LOOP) => Ok(()),
            Err(Errno::NOENT) => {
                let target = readlinkat(&self.root, link, Vec::new())?;
                let target = Path::new(OsStr::from_bytes(target.as_bytes()));
                match stays_inside(link, target) {
                    true => Ok(()),
                    false => Err(Refusal::Outside.into()),
                }
            }
            Err(Errno::XDEV) => Err(Refusal::Outside.into()),
            Err(error) => Err(error.into()),
        }
    }

    pub(super) fn finish(self) -> io::Result<()> {
        for link in &self.links {
            self.check_link(link)?;
        }
        for (path, mode) in self.folders.iter().rev() {
            let dir = self.walk(path, false)?;
            fchmod(&dir, Mode::from_raw_mode((mode & KEPT_MODE) | OWNER_MODE))?;
        }
        Ok(())
    }
}

pub(super) fn finish_file(
    file: &File,
    mode: Option<u32>,
    modified: Option<SystemTime>,
) -> io::Result<()> {
    if let Some(mode) = mode {
        file.set_permissions(kept(mode))?;
    }
    if let Some(modified) = modified {
        file.set_modified(modified)?;
    }
    Ok(())
}
