use std::ffi::OsString;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const PRIVATE_FILE: u32 = 0o600;
const PRIVATE_FOLDER: u32 = 0o700;

static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn sync_folder(folder: &Path) -> io::Result<()> {
    match File::open(folder).and_then(|folder| folder.sync_all()) {
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
            ) =>
        {
            Ok(())
        }
        result => result,
    }
}

fn staging_path(target: &Path) -> PathBuf {
    let mut name = OsString::from(".");
    name.push(target.file_name().unwrap_or_default());
    name.push(format!(
        ".{}-{}.new",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    target.with_file_name(name)
}

fn write_staged(
    staging: &Path,
    contents: &[u8],
    target: &Path,
    mode: Option<u32>,
) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    if let Some(mode) = mode {
        options.mode(mode);
    }
    let mut staged = options.open(staging)?;
    staged.write_all(contents)?;
    if mode.is_none()
        && let Ok(existing) = fs::metadata(target)
    {
        staged.set_permissions(existing.permissions())?;
    }
    staged.sync_all()
}

fn replace(
    path: &Path,
    contents: &[u8],
    mode: Option<u32>,
    folder_mode: Option<u32>,
) -> io::Result<()> {
    if let Some(folder) = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        let mut builder = DirBuilder::new();
        builder.recursive(true);
        if let Some(folder_mode) = folder_mode {
            builder.mode(folder_mode);
        }
        builder.create(folder)?;
    }
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let staging = staging_path(&target);
    let written = write_staged(&staging, contents, &target, mode)
        .and_then(|()| fs::rename(&staging, &target));
    if written.is_err() {
        let _ = fs::remove_file(&staging);
    }
    written?;
    match target.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => sync_folder(folder),
        _ => Ok(()),
    }
}

pub fn replace_file(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    replace(path, contents.as_ref(), None, None)
}

pub fn replace_private_file(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    replace(
        path,
        contents.as_ref(),
        Some(PRIVATE_FILE),
        Some(PRIVATE_FOLDER),
    )
}

/// Reads `path` (empty when missing), hands its text to `change` and writes back what it returns,
/// all while holding an exclusive lock on `<path>.lock`, so processes that change the file this
/// way never lose each other's changes.
pub fn change_file(path: &Path, change: impl FnOnce(&str) -> io::Result<String>) -> io::Result<()> {
    if let Some(folder) = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        fs::create_dir_all(folder)?;
    }
    let mut lock_name = path.as_os_str().to_owned();
    lock_name.push(".lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(PRIVATE_FILE)
        .open(PathBuf::from(lock_name))?;
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive)?;
    let before = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let after = change(&before)?;
    match after == before {
        true => Ok(()),
        false => replace_file(path, after),
    }
}
