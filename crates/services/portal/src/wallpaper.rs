use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use bande_xdg::path_from_file_uri;
use zbus::blocking::Connection;
use zbus::object_server::ObjectServer;
use zbus::zvariant::{ObjectPath, OwnedValue};

use crate::request::{self, CANCELLED, Cancellation, OTHER, SUCCESS};

pub const BACKGROUNDS: &str = "backgrounds";

pub trait WallpaperUi: Send + Sync {
    fn confirm(&self, app_id: &str, picture: &Path, cancellation: &Cancellation) -> bool;
}

#[derive(Clone)]
pub struct Wallpaper {
    pub ui: Arc<dyn WallpaperUi>,
    pub folder: PathBuf,
    pub session: Connection,
}

pub fn keep(picture: &Path, folder: &Path) -> io::Result<PathBuf> {
    let name = picture
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the picture has no name"))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    std::fs::create_dir_all(folder)?;
    let kept = folder.join(format!("{stamp}-{}", name.to_string_lossy()));
    std::fs::copy(picture, &kept)?;
    Ok(kept)
}

impl Wallpaper {
    fn set(&self, app_id: &str, uri: &str, preview: bool, cancellation: &Cancellation) -> u32 {
        let Some(picture) = path_from_file_uri(uri).filter(|path| path.is_file()) else {
            tracing::warn!("{app_id} asked for a desktop picture that is not a file: {uri}");
            return OTHER;
        };
        if preview && !self.ui.confirm(app_id, &picture, cancellation) {
            return CANCELLED;
        }
        let chosen = keep(&picture, &self.folder)
            .map_err(zbus::Error::from)
            .and_then(|kept| bande_wallpaper::set(&self.session, &kept));
        match chosen {
            Ok(()) => SUCCESS,
            Err(error) => {
                tracing::warn!("cannot set the desktop picture for {app_id}: {error}");
                OTHER
            }
        }
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Wallpaper")]
impl Wallpaper {
    #[zbus(name = "SetWallpaperURI")]
    async fn set_wallpaper_uri(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        _parent_window: String,
        uri: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> u32 {
        let preview = options
            .get("show-preview")
            .and_then(|value| bool::try_from(value).ok())
            .unwrap_or(false);
        let wallpaper = self.clone();
        let outcome = request::run(server, &handle, move |cancellation| {
            wallpaper.set(&app_id, &uri, preview, cancellation)
        })
        .await;
        match outcome {
            Ok(Some(response)) => response,
            Ok(None) => CANCELLED,
            Err(_) => OTHER,
        }
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}
