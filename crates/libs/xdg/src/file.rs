use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

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

fn write_staged(staging: &Path, contents: &[u8], target: &Path) -> io::Result<()> {
    let mut staged = File::create_new(staging)?;
    staged.write_all(contents)?;
    if let Ok(existing) = fs::metadata(target) {
        staged.set_permissions(existing.permissions())?;
    }
    staged.sync_all()
}

pub fn replace_file(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut staging = OsString::from(target.as_os_str());
    staging.push(format!(
        ".{}-{}.new",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let staging = Path::new(&staging);
    let written = write_staged(staging, contents.as_ref(), &target)
        .and_then(|()| fs::rename(staging, &target));
    if written.is_err() {
        let _ = fs::remove_file(staging);
    }
    written?;
    match target.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => sync_folder(folder),
        _ => Ok(()),
    }
}
