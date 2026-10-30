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

use image::imageops::FilterType;
use image::{ImageBuffer, ImageFormat, Rgba};
use jiff::civil::DateTime;
use unbound_l10n::Localizer;
use unbound_toolkit::ScreenImage;
use unbound_xdg::{Locale, applications, current_desktops, data_dirs, find_app};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-screenshot.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

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

fn pixels(image: &ScreenImage) -> io::Result<ImageBuffer<Rgba<u8>, &[u8]>> {
    ImageBuffer::from_raw(image.width, image.height, image.rgba.as_slice())
        .ok_or_else(|| io::Error::other("the capture does not fill its size"))
}

pub fn shrunk(image: &ScreenImage, most: u32) -> Option<ScreenImage> {
    let scale = (most as f32 / image.width.max(image.height).max(1) as f32).min(1.0);
    let width = ((image.width as f32 * scale).round() as u32).max(1);
    let height = ((image.height as f32 * scale).round() as u32).max(1);
    let small = image::imageops::resize(&pixels(image).ok()?, width, height, FilterType::Triangle);
    Some(ScreenImage {
        width,
        height,
        rgba: small.into_raw(),
    })
}

pub fn png(image: &ScreenImage) -> io::Result<Vec<u8>> {
    let pixels = pixels(image)?;
    let mut bytes = Vec::new();
    pixels
        .write_to(&mut io::Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(io::Error::other)?;
    Ok(bytes)
}

pub fn save(image: &ScreenImage, dir: &Path, taken: DateTime) -> io::Result<PathBuf> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("lib.rs"),
            include_str!("main.rs"),
            include_str!("area.rs"),
            include_str!("picker.rs"),
            include_str!("service.rs"),
            include_str!("palette.rs"),
            include_str!("markup.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn a_taken_name_gets_a_copy_number() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let stem = "Screenshot 2026-10-29 at 09.05.07";
        fs::write(dir.path().join(format!("{stem}.png")), b"").expect("first");

        assert_eq!(
            free_path(dir.path(), stem, "png"),
            dir.path().join(format!("{stem} (2).png"))
        );
    }

    #[test]
    fn a_saved_screenshot_is_a_png_of_the_capture() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let rgba = [[0x10, 0x20, 0x30, 0xff], [0xf0, 0x80, 0x00, 0xff]].concat();
        let taken = jiff::civil::date(2026, 10, 29).at(9, 5, 7, 0);

        let path = save(
            &ScreenImage {
                width: 2,
                height: 1,
                rgba: rgba.clone(),
            },
            &dir.path().join("Desktop"),
            taken,
        )
        .expect("saved");

        assert_eq!(
            path,
            dir.path()
                .join("Desktop/Screenshot 2026-10-29 at 09.05.07.png")
        );
        let read = image::open(&path).expect("a PNG").to_rgba8();
        assert_eq!(read.dimensions(), (2, 1));
        assert_eq!(read.into_raw(), rgba);
        let leftovers = fs::read_dir(dir.path().join("Desktop"))
            .expect("listing")
            .count();
        assert_eq!(leftovers, 1);
    }
}
