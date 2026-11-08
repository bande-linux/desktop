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

use bande_l10n::tr;
use bande_toolkit::Snapshot;
use bande_xdg::{Locale, applications, current_desktops, data_dirs, find_app};
use image::imageops::FilterType;
use image::{ImageBuffer, ImageFormat, Rgba};
use jiff::civil::DateTime;

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
            Self::NoRecorder => (
                tr!("Screen recording is not available"),
                tr!(
                    "Install gstreamer1.0-pipewire, gstreamer1.0-plugins-good and gstreamer1.0-plugins-ugly to record the screen."
                ),
            ),
            Self::ScreenUnavailable => (
                tr!("Screen recording is not available"),
                tr!("The screen could not be captured. Try again in a moment."),
            ),
            Self::RecordingFailed => (
                tr!("The recording could not be saved"),
                tr!(
                    "Something went wrong while recording. Check that there is enough free disk space and try again."
                ),
            ),
            Self::NotSaved(dir) => (
                tr!("The screenshot could not be saved"),
                tr!(
                    "Bande could not write to {folder}. Choose another place in the options, or check the folder’s permissions.",
                    folder = dir.display().to_string()
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
    let (date, time) = stamp(taken);
    tr!("Screenshot {date} at {time}", date = date, time = time)
}

pub fn recording_stem(taken: DateTime) -> String {
    let (date, time) = stamp(taken);
    tr!(
        "Screen Recording {date} at {time}",
        date = date,
        time = time
    )
}

fn stamp(taken: DateTime) -> (String, String) {
    (
        taken.strftime("%Y-%m-%d").to_string(),
        taken.strftime("%H.%M.%S").to_string(),
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
