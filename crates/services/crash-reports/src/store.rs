use std::error::Error;
use std::fs::{self, File};
use std::io::Write;
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};

use rustix::fs::{
    AtFlags, FlockOperation, Gid, Mode, OFlags, Uid, chown, fchown, flock, mkdirat, openat,
    unlinkat,
};
use rustix::io::Errno;

use crate::report::{Owner, Report};

const SYSTEM_FOLDER: &str = "system";
const ADMIN_GROUP: &str = "sudo";
const KEEP_REPORTS: usize = 200;
const REPORT_MODE: u32 = 0o640;
const FOLDER_MODE: u32 = 0o750;

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn open(root: &Path) -> Result<Self, Box<dyn Error>> {
        fs::create_dir_all(root)?;
        Ok(Self {
            root: root.to_path_buf(),
        })
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    pub fn lock(&self) -> Result<OwnedFd, Box<dyn Error>> {
        let lock = File::create(self.path("lock"))?;
        flock(&lock, FlockOperation::LockExclusive)?;
        Ok(lock.into())
    }

    pub fn read(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.path(name)).ok()
    }

    pub fn replace(&self, name: &str, contents: &str) -> Result<(), Box<dyn Error>> {
        let staged = self.path(&format!("{name}.new"));
        fs::write(&staged, contents)?;
        fs::rename(staged, self.path(name))?;
        Ok(())
    }

    fn folder(&self, owner: Owner) -> Result<(OwnedFd, Uid, Gid), Box<dyn Error>> {
        let (name, uid, gid) = match owner {
            Owner::User(uid) => (uid.to_string(), uid, primary_group(uid).unwrap_or(uid)),
            Owner::System => (
                SYSTEM_FOLDER.to_owned(),
                0,
                group_id(ADMIN_GROUP).unwrap_or(0),
            ),
        };
        let uid = Uid::from_raw(uid);
        let gid = Gid::from_raw(gid);
        let root = rustix::fs::open(
            &self.root,
            OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        match mkdirat(&root, name.as_str(), Mode::from_raw_mode(FOLDER_MODE)) {
            Ok(()) => chown(self.root.join(&name), Some(uid), Some(gid))?,
            Err(Errno::EXIST) => {}
            Err(error) => return Err(error.into()),
        }
        let folder = openat(
            &root,
            name.as_str(),
            OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        Ok((folder, uid, gid))
    }

    pub fn write(&self, report: &Report) -> Result<(), Box<dyn Error>> {
        let (folder, uid, gid) = self.folder(report.owner())?;
        let created = openat(
            &folder,
            report.file_name(),
            OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(REPORT_MODE),
        );
        let file = match created {
            Ok(file) => file,
            Err(Errno::EXIST) => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        fchown(&file, Some(uid), Some(gid))?;
        File::from(file).write_all(report.text().as_bytes())?;
        Ok(())
    }

    pub fn prune(&self) {
        let Ok(folders) = fs::read_dir(&self.root) else {
            return;
        };
        for folder in folders.flatten() {
            if folder.file_type().is_ok_and(|kind| kind.is_dir()) {
                prune_folder(&folder.path());
            }
        }
    }
}

fn prune_folder(path: &Path) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    let mut names: Vec<_> = entries.flatten().map(|entry| entry.file_name()).collect();
    if names.len() <= KEEP_REPORTS {
        return;
    }
    names.sort();
    let Ok(folder) = rustix::fs::open(
        path,
        OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) else {
        return;
    };
    for name in &names[..names.len() - KEEP_REPORTS] {
        if let Err(error) = unlinkat(folder.as_fd(), name.as_os_str(), AtFlags::empty()) {
            tracing::debug!("cannot remove an old report: {error}");
        }
    }
}

fn primary_group(uid: u32) -> Option<u32> {
    let passwd = fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        (fields.get(2)?.parse() == Ok(uid)).then(|| fields.get(3)?.parse().ok())?
    })
}

fn group_id(name: &str) -> Option<u32> {
    let groups = fs::read_to_string("/etc/group").ok()?;
    groups.lines().find_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        (*fields.first()? == name).then(|| fields.get(2)?.parse().ok())?
    })
}
