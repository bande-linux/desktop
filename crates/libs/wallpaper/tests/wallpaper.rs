#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use image::{Rgba, RgbaImage};
use unbound_wallpaper::{Choice, Fit, Variant, backdrop, bundled, render, render_cached};

fn bundled_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let wallpapers = dir.path().join("unbound/wallpapers");
    std::fs::create_dir_all(&wallpapers).unwrap();
    for name in ["drift-light.svg", "drift-dark.svg"] {
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../data/wallpapers")
                .join(name),
            wallpapers.join(name),
        )
        .unwrap();
    }
    dir
}

#[test]
fn every_bundled_wallpaper_has_a_light_and_a_dark_drawing() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data/wallpapers");
    let mut names: Vec<String> = std::fs::read_dir(&data)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert!(!names.is_empty());
    for name in &names {
        let stem = name
            .strip_suffix("-light.svg")
            .or_else(|| name.strip_suffix("-dark.svg"))
            .unwrap_or_else(|| panic!("{name} is neither a light nor a dark drawing"));
        assert!(names.contains(&format!("{stem}-light.svg")));
        assert!(names.contains(&format!("{stem}-dark.svg")));
        let picture = render(&data.join(name), Fit::Fill, 64, 40).unwrap();
        assert_eq!((picture.width, picture.height), (64, 40));
        assert!(picture.rgba.chunks(4).all(|pixel| pixel[3] == 255));
    }
}

#[test]
fn the_default_follows_the_variant_and_a_choice_overrides_it() {
    let dir = bundled_dir();
    let data = vec![PathBuf::from("/nonexistent"), dir.path().to_path_buf()];
    let none = Choice::default();
    assert_eq!(
        none.path(Variant::Dark, None, &data),
        Some(dir.path().join("unbound/wallpapers/drift-dark.svg"))
    );
    assert_eq!(bundled("missing", Variant::Light, &data), None);
    let home = Path::new("/home/ada");
    let one = Choice {
        light: Some(PathBuf::from("~/Pictures/lake.jpg")),
        ..Choice::default()
    };
    assert_eq!(
        one.path(Variant::Dark, Some(home), &data),
        Some(PathBuf::from("/home/ada/Pictures/lake.jpg"))
    );
    let both = Choice {
        light: Some(PathBuf::from("/srv/day.png")),
        dark: Some(PathBuf::from("/srv/night.png")),
        fit: Fit::Fill,
    };
    assert_eq!(
        both.path(Variant::Dark, Some(home), &data),
        Some(PathBuf::from("/srv/night.png"))
    );
    let relative = Choice {
        light: Some(PathBuf::from("lake.jpg")),
        ..Choice::default()
    };
    assert_eq!(
        relative.path(Variant::Light, Some(home), &data),
        Some(dir.path().join("unbound/wallpapers/drift-light.svg"))
    );
}

fn photo(dir: &Path) -> PathBuf {
    let mut image = RgbaImage::from_pixel(40, 20, Rgba([200, 30, 30, 255]));
    for x in 10..30 {
        for y in 0..20 {
            image.put_pixel(x, y, Rgba([20, 20, 220, 255]));
        }
    }
    let path = dir.join("photo.png");
    image.save(&path).unwrap();
    path
}

fn pixel(picture: &unbound_wallpaper::Picture, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * picture.width + x) * 4) as usize;
    picture.rgba[at..at + 4].try_into().unwrap()
}

#[test]
fn fill_crops_the_centre_and_fit_shows_everything_on_the_edge_colour() {
    let dir = tempfile::tempdir().unwrap();
    let path = photo(dir.path());
    let filled = render(&path, Fit::Fill, 20, 20).unwrap();
    assert_eq!((filled.width, filled.height), (20, 20));
    assert_eq!(pixel(&filled, 10, 10), [20, 20, 220, 255]);
    assert!(pixel(&filled, 1, 10)[2] > 150);

    let fitted = render(&path, Fit::Fit, 40, 40).unwrap();
    assert_eq!((fitted.width, fitted.height), (40, 40));
    assert_eq!(pixel(&fitted, 2, 15), [200, 30, 30, 255]);
    assert_eq!(pixel(&fitted, 20, 20), [20, 20, 220, 255]);
    let bar = pixel(&fitted, 20, 2);
    assert_eq!(bar, pixel(&fitted, 20, 37));
    assert!(bar[0] > 100 && bar[2] > 60, "{bar:?}");
}

#[test]
fn the_backdrop_is_small_dim_and_opaque() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("white.png");
    RgbaImage::from_pixel(64, 40, Rgba([255, 255, 255, 255]))
        .save(&path)
        .unwrap();
    let picture = backdrop(&path, Fit::Fill, 640, 400).unwrap();
    assert_eq!((picture.width, picture.height), (64, 40));
    let [red, green, blue, alpha] = pixel(&picture, 32, 20);
    assert_eq!(alpha, 255);
    assert!(red < 180 && red == green && green == blue, "{red}");
}

#[test]
fn a_rendered_wallpaper_is_kept_for_the_next_sign_in_until_the_picture_changes() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    let path = photo(dir.path());
    let first = render_cached(&cache, &path, Fit::Fill, 20, 20).unwrap();
    assert_eq!(first, render(&path, Fit::Fill, 20, 20).unwrap());
    let entries = || {
        std::fs::read_dir(&cache)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>()
    };
    let [entry] = entries().try_into().unwrap();
    let mut marked = std::fs::read(&entry).unwrap();
    marked[8..].fill(7);
    std::fs::write(&entry, &marked).unwrap();
    assert_eq!(
        render_cached(&cache, &path, Fit::Fill, 20, 20)
            .unwrap()
            .rgba,
        vec![7; 20 * 20 * 4],
        "the second sign-in reads the kept pixels"
    );

    RgbaImage::from_pixel(30, 30, Rgba([10, 200, 10, 255]))
        .save(&path)
        .unwrap();
    assert_eq!(
        pixel(
            &render_cached(&cache, &path, Fit::Fill, 20, 20).unwrap(),
            5,
            5
        ),
        [10, 200, 10, 255],
        "a changed picture is drawn again"
    );
    for size in 21..30 {
        render_cached(&cache, &path, Fit::Fill, size, size).unwrap();
    }
    assert_eq!(entries().len(), 4, "only the latest few are kept");
}
