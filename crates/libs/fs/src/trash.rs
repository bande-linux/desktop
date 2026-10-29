use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use jiff::civil::DateTime;
use unbound_xdg::{percent_decode, percent_encode};

const INFO_SUFFIX: &str = ".trashinfo";
const INFO_HEADER: &str = "[Trash Info]";
const STICKY: u32 = 0o1000;
const DATE_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashDir {
    root: PathBuf,
    topdir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashItem {
    pub trash: TrashDir,
    pub id: OsString,
    pub original: PathBuf,
    pub deleted: Option<DateTime>,
}

impl TrashItem {
    pub fn path(&self) -> PathBuf {
        self.trash.files().join(&self.id)
    }

    fn info(&self) -> PathBuf {
        self.trash.info_path(&self.id)
    }

    pub fn name(&self) -> String {
        self.original.file_name().map_or_else(
            || self.id.to_string_lossy().into_owned(),
            |name| name.to_string_lossy().into_owned(),
        )
    }
}

fn not_found(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, message)
}

fn create_private_dir(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

impl TrashDir {
    pub fn home(root: PathBuf) -> Self {
        Self { root, topdir: None }
    }

    pub fn for_volume(topdir: &Path, uid: u32) -> io::Result<Self> {
        let shared = topdir.join(".Trash");
        let usable_shared = fs::symlink_metadata(&shared)
            .is_ok_and(|metadata| metadata.is_dir() && metadata.permissions().mode() & STICKY != 0);
        if usable_shared {
            let root = shared.join(uid.to_string());
            if create_private_dir(&root).is_ok()
                && fs::symlink_metadata(&root).is_ok_and(|metadata| metadata.is_dir())
            {
                return Ok(Self {
                    root,
                    topdir: Some(topdir.to_path_buf()),
                });
            }
        }
        let root = topdir.join(format!(".Trash-{uid}"));
        create_private_dir(&root)?;
        let metadata = fs::symlink_metadata(&root)?;
        if !metadata.is_dir() || metadata.uid() != uid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the volume's trash folder is not usable",
            ));
        }
        Ok(Self {
            root,
            topdir: Some(topdir.to_path_buf()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn files(&self) -> PathBuf {
        self.root.join("files")
    }

    fn info_dir(&self) -> PathBuf {
        self.root.join("info")
    }

    fn info_path(&self, id: &OsString) -> PathBuf {
        let mut name = id.clone();
        name.push(INFO_SUFFIX);
        self.info_dir().join(name)
    }

    fn stored_path(&self, original: &Path) -> PathBuf {
        self.topdir
            .as_deref()
            .and_then(|topdir| original.strip_prefix(topdir).ok())
            .map_or_else(|| original.to_path_buf(), Path::to_path_buf)
    }

    fn original_path(&self, stored: PathBuf) -> PathBuf {
        match &self.topdir {
            Some(topdir) if stored.is_relative() => topdir.join(stored),
            _ => stored,
        }
    }

    fn reserve(&self, original: &Path, deleted: DateTime) -> io::Result<(OsString, PathBuf)> {
        create_private_dir(&self.info_dir())?;
        create_private_dir(&self.files())?;
        let name = original
            .file_name()
            .ok_or_else(|| not_found("the item has no name"))?
            .to_os_string();
        let contents = format!(
            "{INFO_HEADER}\nPath={}\nDeletionDate={}\n",
            percent_encode(&self.stored_path(original)),
            deleted.strftime(DATE_FORMAT)
        );
        for attempt in 1u64.. {
            let mut id = name.clone();
            if attempt > 1 {
                id.push(format!(".{attempt}"));
            }
            let info = self.info_path(&id);
            if self.files().join(&id).symlink_metadata().is_ok() {
                continue;
            }
            match OpenOptions::new().write(true).create_new(true).open(&info) {
                Ok(mut file) => {
                    file.write_all(contents.as_bytes())?;
                    return Ok((id, info));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other("no free name in the trash"))
    }

    pub fn put(&self, path: &Path, deleted: DateTime) -> io::Result<TrashItem> {
        let deleted = deleted
            .with()
            .subsec_nanosecond(0)
            .build()
            .unwrap_or(deleted);
        let (id, info) = self.reserve(path, deleted)?;
        if let Err(error) = fs::rename(path, self.files().join(&id)) {
            let _ = fs::remove_file(info);
            return Err(error);
        }
        Ok(TrashItem {
            trash: self.clone(),
            id,
            original: path.to_path_buf(),
            deleted: Some(deleted),
        })
    }

    pub fn items(&self) -> Vec<TrashItem> {
        let Ok(entries) = fs::read_dir(self.info_dir()) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let file_name = entry.file_name();
                let id = file_name.as_bytes().strip_suffix(INFO_SUFFIX.as_bytes())?;
                let id = OsString::from(std::ffi::OsStr::from_bytes(id));
                let text = fs::read_to_string(entry.path()).ok()?;
                let (stored, deleted) = parse_info(&text)?;
                self.files().join(&id).symlink_metadata().ok()?;
                Some(TrashItem {
                    trash: self.clone(),
                    original: self.original_path(stored),
                    id,
                    deleted,
                })
            })
            .collect()
    }
}

fn parse_info(text: &str) -> Option<(PathBuf, Option<DateTime>)> {
    let mut lines = text.lines().map(str::trim);
    lines.find(|line| *line == INFO_HEADER)?;
    let mut path = None;
    let mut deleted = None;
    for line in lines.take_while(|line| !line.starts_with('[')) {
        if let Some(value) = line.strip_prefix("Path=") {
            path = Some(percent_decode(value));
        } else if let Some(value) = line.strip_prefix("DeletionDate=") {
            deleted = DateTime::strptime(DATE_FORMAT, value).ok();
        }
    }
    Some((path?, deleted))
}

pub fn remove_all(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

#[derive(Debug, Clone)]
pub struct Trash {
    home: TrashDir,
    uid: u32,
}

fn mount_points() -> Vec<PathBuf> {
    fs::read_to_string("/proc/self/mounts")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split(' ').nth(1))
        .map(unescape_mount_field)
        .collect()
}

fn unescape_mount_field(field: &str) -> PathBuf {
    let bytes = field.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let octal = (bytes[index] == b'\\')
            .then(|| bytes.get(index + 1..index + 4))
            .flatten()
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u8::from_str_radix(digits, 8).ok());
        match octal {
            Some(byte) => {
                decoded.push(byte);
                index += 4;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }
    PathBuf::from(std::ffi::OsStr::from_bytes(&decoded))
}

fn topdir(path: &Path) -> io::Result<PathBuf> {
    let device = fs::symlink_metadata(path)?.dev();
    let mut top = path
        .parent()
        .ok_or_else(|| not_found("the item has no folder"))?
        .to_path_buf();
    while let Some(parent) = top.parent() {
        if fs::metadata(parent)?.dev() != device {
            break;
        }
        top = parent.to_path_buf();
    }
    Ok(top)
}

impl Trash {
    pub fn new(home: PathBuf, uid: u32) -> Self {
        Self {
            home: TrashDir::home(home),
            uid,
        }
    }

    pub fn for_user() -> Option<Self> {
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/share"))
            })?;
        Some(Self::new(
            data_home.join("Trash"),
            rustix::process::getuid().as_raw(),
        ))
    }

    pub fn home(&self) -> &TrashDir {
        &self.home
    }

    fn home_device(&self) -> Option<u64> {
        let mut probe = self.home.root.as_path();
        loop {
            if let Ok(metadata) = fs::metadata(probe) {
                return Some(metadata.dev());
            }
            probe = probe.parent()?;
        }
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.dirs().iter().any(|dir| path.starts_with(&dir.root))
    }

    fn dir_for(&self, path: &Path) -> io::Result<TrashDir> {
        let device = fs::symlink_metadata(path)?.dev();
        if self.home_device() == Some(device) {
            return Ok(self.home.clone());
        }
        TrashDir::for_volume(&topdir(path)?, self.uid)
    }

    pub fn put(&self, path: &Path) -> io::Result<TrashItem> {
        if self.contains(path) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the item is already in the trash",
            ));
        }
        let path = std::path::absolute(path)?;
        self.dir_for(&path)?
            .put(&path, jiff::Zoned::now().datetime())
    }

    fn dirs(&self) -> Vec<TrashDir> {
        let uid = self.uid.to_string();
        let mut dirs = vec![self.home.clone()];
        for topdir in mount_points() {
            for root in [
                topdir.join(".Trash").join(&uid),
                topdir.join(format!(".Trash-{uid}")),
            ] {
                if root != self.home.root && root.join("info").is_dir() {
                    dirs.push(TrashDir {
                        root,
                        topdir: Some(topdir.clone()),
                    });
                }
            }
        }
        dirs
    }

    pub fn items(&self) -> Vec<TrashItem> {
        self.dirs().iter().flat_map(TrashDir::items).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.items().is_empty()
    }

    pub fn empty(&self) -> io::Result<()> {
        self.items().iter().try_for_each(erase)
    }
}

pub fn restore_to(item: &TrashItem, destination: &Path) -> io::Result<()> {
    if destination.symlink_metadata().is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "an item with that name already exists",
        ));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(item.path(), destination)?;
    forget(item)
}

pub fn forget(item: &TrashItem) -> io::Result<()> {
    fs::remove_file(item.info())
}

pub fn erase(item: &TrashItem) -> io::Result<()> {
    match remove_all(&item.path()) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    forget(item)
}
