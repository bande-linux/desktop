use std::fs::{File, OpenOptions, Permissions};
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread;

use crate::{Accessibility, Config, Setting, Wallpaper, WallpaperVariant};

pub const LOGIN_PICTURES: &str = "/var/lib/bande-greeter/pictures";
pub const LOGIN_PICTURE_LIMIT: u64 = 64 << 20;
pub const LOGIN_LOOK_LIMIT: u64 = 16 << 10;
const SHARED_MODE: u32 = 0o640;

static LATEST_SHARE: AtomicU64 = AtomicU64::new(0);
static SHARING: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq)]
pub struct LoginLook {
    pub dark: Option<PathBuf>,
    pub light: Option<PathBuf>,
    pub look: String,
}

fn expanded(path: &Path, home: Option<&Path>) -> Option<PathBuf> {
    match path.strip_prefix("~") {
        Ok(rest) => home.map(|home| home.join(rest)),
        Err(_) => Some(path.to_path_buf()),
    }
}

fn look_of(config: &Config) -> String {
    let access = config.accessibility;
    let shared = Accessibility {
        screen_reader: access.screen_reader,
        increase_contrast: access.increase_contrast,
        reduce_transparency: access.reduce_transparency,
        reduce_motion: access.reduce_motion,
        text_size: access.text_size,
        ..Accessibility::default()
    };
    [
        Setting::Appearance(config.appearance),
        Setting::WallpaperFit(config.wallpaper.fit),
        Setting::Accessibility(shared),
    ]
    .iter()
    .try_fold(String::new(), |source, setting| setting.apply(&source))
    .unwrap_or_default()
}

pub fn login_look(config: &Config, home: Option<&Path>) -> LoginLook {
    let wallpaper: &Wallpaper = &config.wallpaper;
    let path = |variant| {
        wallpaper
            .path(variant)
            .and_then(|path| expanded(path, home))
    };
    let dark = path(WallpaperVariant::Dark).or_else(|| path(WallpaperVariant::Light));
    let light = path(WallpaperVariant::Light).filter(|light| Some(light) != dark.as_ref());
    LoginLook {
        dark,
        light,
        look: look_of(config),
    }
}

pub fn login_picture_file(folder: &Path, uid: u32) -> PathBuf {
    folder.join(uid.to_string())
}

pub fn login_light_picture_file(folder: &Path, uid: u32) -> PathBuf {
    folder.join(format!("{uid}.light"))
}

pub fn login_look_file(folder: &Path, uid: u32) -> PathBuf {
    folder.join(format!("{uid}.look"))
}

fn share(shared: &Path, source: Option<(&mut dyn Read, u64)>, limit: u64) -> io::Result<()> {
    let Some((source, length)) = source else {
        return match std::fs::remove_file(shared) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
    };
    if length > limit {
        return Err(io::Error::other(format!(
            "{} would be larger than {limit} bytes",
            shared.display()
        )));
    }
    let name = shared.file_name().unwrap_or_default().to_string_lossy();
    let staging = shared.with_file_name(format!(".{name}.{}", std::process::id()));
    let copied = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(SHARED_MODE)
        .open(&staging)
        .and_then(|mut copy| {
            copy.set_permissions(Permissions::from_mode(SHARED_MODE))?;
            io::copy(&mut source.take(limit), &mut copy)?;
            copy.sync_all()
        })
        .and_then(|()| std::fs::rename(&staging, shared));
    if copied.is_err() {
        let _ = std::fs::remove_file(&staging);
    }
    copied
}

fn share_picture(shared: &Path, picture: Option<&Path>) -> io::Result<()> {
    let Some(picture) = picture else {
        return share(shared, None, LOGIN_PICTURE_LIMIT);
    };
    let mut source = File::open(picture)?;
    let length = source.metadata()?.len();
    share(shared, Some((&mut source, length)), LOGIN_PICTURE_LIMIT)
}

pub fn copy_login_look(folder: &Path, uid: u32, look: &LoginLook) -> io::Result<()> {
    share_picture(&login_picture_file(folder, uid), look.dark.as_deref())?;
    share_picture(
        &login_light_picture_file(folder, uid),
        look.light.as_deref(),
    )?;
    let mut text = look.look.as_bytes();
    let length = text.len() as u64;
    share(
        &login_look_file(folder, uid),
        Some((&mut text, length)),
        LOGIN_LOOK_LIMIT,
    )
}

pub fn share_login_look(look: LoginLook) {
    let share = LATEST_SHARE.fetch_add(1, Ordering::SeqCst) + 1;
    let shared = thread::Builder::new()
        .name("login-look".to_owned())
        .spawn(move || {
            let _sharing = SHARING.lock().unwrap_or_else(PoisonError::into_inner);
            if LATEST_SHARE.load(Ordering::SeqCst) != share {
                return;
            }
            let copied = std::fs::metadata("/proc/self").and_then(|process| {
                copy_login_look(Path::new(LOGIN_PICTURES), process.uid(), &look)
            });
            if let Err(error) = copied {
                tracing::info!("the login window keeps its own look: {error}");
            }
        });
    if let Err(error) = shared {
        tracing::warn!("cannot share the look with the login window: {error}");
    }
}
