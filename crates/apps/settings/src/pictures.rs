use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};

use bande_app::choose_file;
use bande_wallpaper::{Fit, Picture, render};
use zbus::blocking::Connection;

use crate::failure::unexpected;
use crate::follow::{Update, Updates, spawn};
use crate::{L10N, SettingsClient};
use bande_settings_panes::{SettingId, Value as SettingValue};

pub const THUMBNAIL_SIZE: (u32, u32) = (288, 180);
const IMAGE_TYPES: [&str; 5] = [
    "image/jpeg",
    "image/png",
    "image/webp",
    "image/svg+xml",
    "image/bmp",
];

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Pair {
    pub light: PathBuf,
    pub dark: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PictureCommand {
    Thumbnail(Pair),
    Choose,
}

pub fn bundled_pairs(data_dirs: &[PathBuf]) -> Vec<(String, Pair)> {
    let mut pairs: Vec<(String, Pair)> = Vec::new();
    for dir in data_dirs {
        let folder = dir.join("bande/wallpapers");
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        let mut found = entries
            .flatten()
            .filter_map(|entry| {
                let file = entry.file_name().into_string().ok()?;
                let name = file.strip_suffix("-light.svg")?.to_owned();
                let dark = folder.join(format!("{name}-dark.svg"));
                dark.is_file().then(|| {
                    (
                        name,
                        Pair {
                            light: entry.path(),
                            dark,
                        },
                    )
                })
            })
            .filter(|(name, _)| !pairs.iter().any(|(known, _)| known == name))
            .collect::<Vec<_>>();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        pairs.extend(found);
    }
    pairs
}

pub fn expand(path: &str, home: Option<&Path>) -> Option<PathBuf> {
    match path.strip_prefix("~/") {
        Some(rest) => home.map(|home| home.join(rest)),
        None => Path::new(path).is_absolute().then(|| PathBuf::from(path)),
    }
}

fn side_by_side(light: &Picture, dark: &Picture) -> Picture {
    let mut rgba = light.rgba.clone();
    let half = light.width / 2;
    if (dark.width, dark.height) == (light.width, light.height) {
        for row in 0..light.height {
            let start = ((row * light.width + half) * 4) as usize;
            let end = ((row + 1) * light.width * 4) as usize;
            rgba[start..end].copy_from_slice(&dark.rgba[start..end]);
        }
    }
    Picture {
        width: light.width,
        height: light.height,
        rgba,
    }
}

pub fn thumbnail(pair: &Pair) -> Option<Picture> {
    let (width, height) = THUMBNAIL_SIZE;
    let light = render(&pair.light, Fit::Fill, width, height)
        .inspect_err(|error| tracing::warn!("{}: {error}", pair.light.display()))
        .ok()?;
    if pair.dark == pair.light {
        return Some(light);
    }
    let dark = render(&pair.dark, Fit::Fill, width, height).ok()?;
    Some(side_by_side(&light, &dark))
}

fn choose_picture(session: &Connection) -> zbus::Result<Option<PathBuf>> {
    let filter = (
        L10N.tr("pictures"),
        IMAGE_TYPES
            .iter()
            .map(|mime| (1u32, (*mime).to_owned()))
            .collect::<Vec<_>>(),
    );
    choose_file(session, &L10N.tr("choose-picture"), Some(filter))
}

pub fn follow_pictures(
    session: &Connection,
    data_dirs: Vec<PathBuf>,
    updates: Updates,
) -> Sender<PictureCommand> {
    let (sender, receiver) = mpsc::channel::<PictureCommand>();
    let session = session.clone();
    spawn("wallpaper-pictures", move || {
        let pairs = bundled_pairs(&data_dirs);
        if !updates.send(Update::Pictures(pairs.clone())) {
            return;
        }
        for (_, pair) in &pairs {
            if let Some(picture) = thumbnail(pair)
                && !updates.send(Update::Thumbnail(pair.clone(), picture))
            {
                return;
            }
        }
        let client = SettingsClient::connect(&session);
        for command in receiver {
            match command {
                PictureCommand::Thumbnail(pair) => {
                    if let Some(picture) = thumbnail(&pair) {
                        updates.send(Update::Thumbnail(pair, picture));
                    }
                }
                PictureCommand::Choose => {
                    let chosen = match choose_picture(&session) {
                        Ok(chosen) => chosen,
                        Err(error) => {
                            updates.send(unexpected(error));
                            continue;
                        }
                    };
                    let Some(path) = chosen else {
                        continue;
                    };
                    let path = path.to_string_lossy().into_owned();
                    let value = SettingValue::Pictures {
                        light: path.clone(),
                        dark: path,
                    };
                    let result = client
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|client| client.set(SettingId::Wallpaper, &value));
                    if let Err(error) = result {
                        updates.send(unexpected(error));
                    }
                }
            }
        }
    });
    sender
}
