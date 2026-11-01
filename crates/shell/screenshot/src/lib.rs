pub mod area;
pub mod cast;
pub mod color;
pub mod engine;
pub mod indicator;
pub mod markup;
pub mod options;
pub mod palette;
pub mod picker;
pub mod record;
pub mod service;
pub mod shadow;
pub mod thumbnail;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use bande_l10n::Localizer;
use bande_toolkit::Snapshot;
use bande_xdg::{Locale, applications, current_desktops, data_dirs, find_app};
use image::imageops::FilterType;
use image::{ImageBuffer, ImageFormat, Rgba};
use jiff::civil::DateTime;

const ENGLISH: &str = include_str!("../locales/en-US/bande-screenshot.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    NoRecorder,
    ScreenUnavailable,
    RecordingFailed,
    NotSaved(PathBuf),
}

impl Failure {
    pub fn message(&self) -> (String, String) {
        match self {
            Self::NoRecorder => (L10N.tr("cannot-record"), L10N.tr("no-recorder")),
            Self::ScreenUnavailable => (L10N.tr("cannot-record"), L10N.tr("screen-unavailable")),
            Self::RecordingFailed => (
                L10N.tr("recording-failed"),
                L10N.tr("recording-failed-detail"),
            ),
            Self::NotSaved(dir) => (
                L10N.tr("cannot-save"),
                L10N.tr_args(
                    "cannot-save-detail",
                    [("folder", dir.display().to_string().into())],
                ),
            ),
        }
    }
}

pub fn app_name(app_id: &str) -> Option<String> {
    if app_id.is_empty() {
        return None;
    }
    let locale = Locale::from_env();
    let entries = applications(&data_dirs(), locale.as_ref(), &current_desktops());
    find_app(&entries, app_id).map(|entry| entry.name.clone())
}

pub fn file_stem(taken: DateTime) -> String {
    stem("file-name", taken)
}

pub fn recording_stem(taken: DateTime) -> String {
    stem("recording-name", taken)
}

fn stem(id: &str, taken: DateTime) -> String {
    L10N.tr_args(
        id,
        [
            ("date", taken.strftime("%Y-%m-%d").to_string().into()),
            ("time", taken.strftime("%H.%M.%S").to_string().into()),
        ],
    )
}

pub fn free_path(dir: &Path, stem: &str, extension: &str) -> PathBuf {
    let candidate = |copy: u32| match copy {
        1 => dir.join(format!("{stem}.{extension}")),
        _ => dir.join(format!("{stem} ({copy}).{extension}")),
    };
    (1..)
        .map(candidate)
        .find(|path| !path.exists())
        .unwrap_or_else(|| candidate(1))
}

fn pixels(image: &Snapshot) -> io::Result<ImageBuffer<Rgba<u8>, &[u8]>> {
    ImageBuffer::from_raw(image.width, image.height, image.rgba.as_slice())
        .ok_or_else(|| io::Error::other("the capture does not fill its size"))
}

pub fn shrunk(image: &Snapshot, most: u32) -> Option<Snapshot> {
    let scale = (most as f32 / image.width.max(image.height).max(1) as f32).min(1.0);
    let width = ((image.width as f32 * scale).round() as u32).max(1);
    let height = ((image.height as f32 * scale).round() as u32).max(1);
    let small = image::imageops::resize(&pixels(image).ok()?, width, height, FilterType::Triangle);
    Some(Snapshot {
        width,
        height,
        rgba: small.into_raw(),
    })
}

pub fn png(image: &Snapshot) -> io::Result<Vec<u8>> {
    let pixels = pixels(image)?;
    let mut bytes = Vec::new();
    pixels
        .write_to(&mut io::Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(io::Error::other)?;
    Ok(bytes)
}

pub fn save(image: &Snapshot, dir: &Path, taken: DateTime) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = free_path(dir, &file_stem(taken), "png");
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("a screenshot path without a name"))?;
    let partial = dir.join(format!(".{}", name.to_string_lossy()));
    pixels(image)?
        .save_with_format(&partial, ImageFormat::Png)
        .map_err(io::Error::other)?;
    fs::rename(&partial, &path)?;
    Ok(path)
}
