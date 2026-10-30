use std::fs::{File, OpenOptions, Permissions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread;

use crate::{Wallpaper, WallpaperVariant};

pub const LOGIN_PICTURES: &str = "/var/lib/unbound-greeter/pictures";
pub const LOGIN_PICTURE_LIMIT: u64 = 64 << 20;
const SHARED_MODE: u32 = 0o640;

static LATEST_SHARE: AtomicU64 = AtomicU64::new(0);
static SHARING: Mutex<()> = Mutex::new(());

pub fn login_picture(wallpaper: &Wallpaper, home: Option<&Path>) -> Option<PathBuf> {
    let path = wallpaper
        .path(WallpaperVariant::Dark)
        .or_else(|| wallpaper.path(WallpaperVariant::Light))?;
    match path.strip_prefix("~") {
        Ok(rest) => home.map(|home| home.join(rest)),
        Err(_) => Some(path.to_path_buf()),
    }
}

pub fn login_picture_file(folder: &Path, uid: u32) -> PathBuf {
    folder.join(uid.to_string())
}

pub fn copy_login_picture(folder: &Path, uid: u32, picture: Option<&Path>) -> io::Result<()> {
    let shared = login_picture_file(folder, uid);
    let Some(picture) = picture else {
        return match std::fs::remove_file(&shared) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
    };
    let mut source = File::open(picture)?;
    if source.metadata()?.len() > LOGIN_PICTURE_LIMIT {
        return Err(io::Error::other(format!(
            "{} is larger than {LOGIN_PICTURE_LIMIT} bytes",
            picture.display()
        )));
    }
    let staging = folder.join(format!(".{uid}.{}", std::process::id()));
    let copied = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(SHARED_MODE)
        .open(&staging)
        .and_then(|mut copy| {
            copy.set_permissions(Permissions::from_mode(SHARED_MODE))?;
            io::copy(&mut source, &mut copy)?;
            copy.sync_all()
        })
        .and_then(|()| std::fs::rename(&staging, &shared));
    if copied.is_err() {
        let _ = std::fs::remove_file(&staging);
    }
    copied
}

pub fn share_login_picture(wallpaper: &Wallpaper) {
    let picture = login_picture(wallpaper, std::env::home_dir().as_deref());
    let share = LATEST_SHARE.fetch_add(1, Ordering::SeqCst) + 1;
    let shared = thread::Builder::new()
        .name("login-picture".to_owned())
        .spawn(move || {
            let _sharing = SHARING.lock().unwrap_or_else(PoisonError::into_inner);
            if LATEST_SHARE.load(Ordering::SeqCst) != share {
                return;
            }
            let copied = std::fs::metadata("/proc/self").and_then(|process| {
                copy_login_picture(Path::new(LOGIN_PICTURES), process.uid(), picture.as_deref())
            });
            if let Err(error) = copied {
                tracing::info!("the login window keeps its own wallpaper: {error}");
            }
        });
    if let Err(error) = shared {
        tracing::warn!("cannot share the wallpaper with the login window: {error}");
    }
}
