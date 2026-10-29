pub mod area;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use image::{ImageFormat, RgbaImage};
use jiff::civil::DateTime;
use unbound_l10n::Localizer;
use unbound_toolkit::ScreenImage;

const ENGLISH: &str = include_str!("../locales/en-US/unbound-screenshot.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub fn file_stem(taken: DateTime) -> String {
    L10N.tr_args(
        "file-name",
        [
            ("date", taken.strftime("%Y-%m-%d").to_string().into()),
            ("time", taken.strftime("%H.%M.%S").to_string().into()),
        ],
    )
}

pub fn free_path(dir: &Path, stem: &str) -> PathBuf {
    let candidate = |copy: u32| match copy {
        1 => dir.join(format!("{stem}.png")),
        _ => dir.join(format!("{stem} ({copy}).png")),
    };
    (1..)
        .map(candidate)
        .find(|path| !path.exists())
        .unwrap_or_else(|| candidate(1))
}

pub fn save(image: ScreenImage, dir: &Path, taken: DateTime) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = free_path(dir, &file_stem(taken));
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("a screenshot path without a name"))?;
    let partial = dir.join(format!(".{}", name.to_string_lossy()));
    let pixels = RgbaImage::from_raw(image.width, image.height, image.rgba)
        .ok_or_else(|| io::Error::other("the capture does not fill its size"))?;
    pixels
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
        ];
        assert_eq!(
            unbound_l10n::missing_ids(ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn the_name_is_the_moment_it_was_taken_as_on_a_mac() {
        let taken = jiff::civil::date(2026, 10, 29).at(9, 5, 7, 0);

        assert_eq!(file_stem(taken), "Screenshot 2026-10-29 at 09.05.07");
    }

    #[test]
    fn a_taken_name_gets_a_copy_number() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let stem = "Screenshot 2026-10-29 at 09.05.07";
        fs::write(dir.path().join(format!("{stem}.png")), b"").expect("first");

        assert_eq!(
            free_path(dir.path(), stem),
            dir.path().join(format!("{stem} (2).png"))
        );
    }

    #[test]
    fn a_saved_screenshot_is_a_png_of_the_capture() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let rgba = [[0x10, 0x20, 0x30, 0xff], [0xf0, 0x80, 0x00, 0xff]].concat();
        let taken = jiff::civil::date(2026, 10, 29).at(9, 5, 7, 0);

        let path = save(
            ScreenImage {
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
