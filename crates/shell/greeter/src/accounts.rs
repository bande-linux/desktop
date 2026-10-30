use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use unbound_config::{LOGIN_PICTURE_LIMIT, login_picture_file};

const LAST_USER: &str = "unbound-greeter/last-user";

pub fn login_picture(folder: &Path, uid: u32) -> Option<File> {
    let path = login_picture_file(folder, uid);
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .inspect_err(|error| tracing::info!("no wallpaper shared by {uid}: {error}"))
        .ok()?;
    let metadata = file.metadata().ok()?;
    let trusted =
        metadata.is_file() && metadata.uid() == uid && metadata.len() <= LOGIN_PICTURE_LIMIT;
    if !trusted {
        tracing::warn!("{} is not a picture {uid} shared", path.display());
    }
    trusted.then_some(file)
}

pub fn opened_path(file: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

pub fn last_user_file(state_home: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let state = match state_home.filter(|path| path.is_absolute()) {
        Some(state) => state.to_path_buf(),
        None => home?.join(".local/state"),
    };
    Some(state.join(LAST_USER))
}

pub fn remember_user(file: &Path, login: &str) -> std::io::Result<()> {
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)?;
    }
    std::fs::write(file, login)
}

pub fn remembered_user(file: &Path) -> Option<String> {
    std::fs::read_to_string(file)
        .ok()
        .map(|login| login.trim().to_owned())
        .filter(|login| !login.is_empty())
}
