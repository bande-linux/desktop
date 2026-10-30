use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::thread;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;

use crate::{Wallpaper, WallpaperVariant};

const ACCOUNTS: &str = "org.freedesktop.Accounts";
const ACCOUNTS_PATH: &str = "/org/freedesktop/Accounts";
const USER: &str = "org.freedesktop.Accounts.User";

pub fn login_picture(wallpaper: &Wallpaper, home: Option<&Path>) -> Option<PathBuf> {
    let path = wallpaper
        .path(WallpaperVariant::Dark)
        .or_else(|| wallpaper.path(WallpaperVariant::Light))?;
    match path.strip_prefix("~") {
        Ok(rest) => home.map(|home| home.join(rest)),
        Err(_) => Some(path.to_path_buf()),
    }
}

fn set_background(picture: &str) -> zbus::Result<()> {
    let uid = std::fs::metadata("/proc/self")
        .map_err(|error| zbus::Error::Failure(error.to_string()))?
        .uid();
    let system = Connection::system()?;
    let accounts = Proxy::new(&system, ACCOUNTS, ACCOUNTS_PATH, ACCOUNTS)?;
    let user: OwnedObjectPath = accounts.call("FindUserById", &(i64::from(uid),))?;
    Proxy::new(&system, ACCOUNTS, user, USER)?.call::<_, _, ()>("SetBackgroundFile", &(picture,))
}

pub fn share_login_picture(wallpaper: &Wallpaper) {
    let home = std::env::home_dir();
    let picture = login_picture(wallpaper, home.as_deref())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let shared = thread::Builder::new()
        .name("login-picture".to_owned())
        .spawn(move || {
            if let Err(error) = set_background(&picture) {
                tracing::info!("the login window keeps its own wallpaper: {error}");
            }
        });
    if let Err(error) = shared {
        tracing::warn!("cannot share the wallpaper with the login window: {error}");
    }
}
