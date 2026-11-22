use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use bande_config::{
    Accessibility, Config, LOGIN_LOOK_LIMIT, LOGIN_PICTURE_LIMIT, login_accessibility_file,
    login_defaults, login_light_picture_file, login_look_file, login_picture_file, login_share,
    read_accessibility,
};
use bande_l10n::tr;

const LAST_USER: &str = "bande-greeter/last-user";
const FAILURE_LIMIT: u64 = 64;

fn shared_file(path: &Path, uid: u32, limit: u64) -> Option<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .inspect_err(|error| {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::info!("cannot open {}: {error}", path.display());
            }
        })
        .ok()?;
    let metadata = file
        .metadata()
        .inspect_err(|error| tracing::warn!("cannot check {}: {error}", path.display()))
        .ok()?;
    let trusted = metadata.is_file() && metadata.uid() == uid && metadata.len() <= limit;
    if !trusted {
        tracing::warn!("{} is not a file {uid} shared", path.display());
    }
    trusted.then_some(file)
}

fn shared_text(path: &Path, uid: u32, limit: u64) -> Option<String> {
    let mut text = String::new();
    shared_file(path, uid, limit)?
        .take(limit)
        .read_to_string(&mut text)
        .inspect_err(|error| tracing::warn!("cannot read {}: {error}", path.display()))
        .ok()?;
    Some(text)
}

pub fn login_picture(folder: &Path, uid: u32, light: bool) -> Option<File> {
    let shared = |path: PathBuf| shared_file(&path, uid, LOGIN_PICTURE_LIMIT);
    match light {
        true => shared(login_light_picture_file(folder, uid)),
        false => None,
    }
    .or_else(|| shared(login_picture_file(folder, uid)))
}

pub fn login_look(folder: &Path, uid: u32) -> Option<Config> {
    let path = login_look_file(folder, uid);
    let text = shared_text(&path, uid, LOGIN_LOOK_LIMIT)?;
    Config::parse(&text)
        .inspect_err(|error| tracing::warn!("{} is not valid: {error}", path.display()))
        .ok()
}

pub fn failed_session(folder: &Path, uid: u32) -> Option<String> {
    let text = shared_text(&login_share(folder, uid).join("failed"), uid, FAILURE_LIMIT)?;
    match text.trim() {
        "already-running" => Some(tr!(
            "Your last sign-in stopped because a session of yours was still running. Try again in a moment."
        )),
        "quick" => Some(tr!(
            "Your last session ended as soon as it started. Try signing in again."
        )),
        "failed" => Some(tr!(
            "Your last session ended unexpectedly. Anything not saved may be lost."
        )),
        other => {
            tracing::warn!("unknown session failure {other:?}");
            None
        }
    }
}

pub fn login_accessibility() -> Accessibility {
    bande_xdg::runtime_dir()
        .and_then(|runtime| read_accessibility(&login_accessibility_file(&runtime)))
        .unwrap_or_else(login_defaults)
}

pub fn last_user_file() -> Option<PathBuf> {
    bande_xdg::state_home().map(|state| state.join(LAST_USER))
}

pub fn remember_user(file: &Path, login: &str) -> std::io::Result<()> {
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)?;
    }
    std::fs::write(file, login)
}

pub fn remembered_user(file: &Path) -> Option<String> {
    std::fs::read_to_string(file)
        .inspect_err(|error| {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!("cannot read {}: {error}", file.display());
            }
        })
        .ok()
        .map(|login| login.trim().to_owned())
        .filter(|login| !login.is_empty())
}
