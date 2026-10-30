#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use unbound_toolkit::{Color, CpuRenderer, Image, Item, Rect, Scene};
use unbound_xdg::{ICON_THEMES, find_icon};

const SIZES: [u32; 5] = [16, 24, 32, 48, 64];
const GAP: f32 = 8.0;
const COLUMNS: usize = 5;
const SYMBOLIC_INK: &str = "#201E1A";
const MIN_SSIM: f64 = 0.99;

fn data() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data")
}

fn theme() -> PathBuf {
    data().join("icons/Unbound")
}

fn directories(theme: &Path) -> Vec<String> {
    let index = fs::read_to_string(theme.join("index.theme")).unwrap();
    index
        .lines()
        .find_map(|line| line.strip_prefix("Directories="))
        .unwrap()
        .split(',')
        .map(str::to_owned)
        .collect()
}

fn icons(theme: &Path) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut icons = Vec::new();
    for directory in directories(theme) {
        if !directory.starts_with("scalable") && !directory.starts_with("symbolic") {
            continue;
        }
        let mut paths: Vec<PathBuf> = fs::read_dir(theme.join(&directory))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        paths.sort();
        for path in paths {
            if seen.insert(fs::read(&path).unwrap()) {
                icons.push(path);
            }
        }
    }
    icons
}

fn name(path: &Path) -> &str {
    path.file_stem().and_then(|name| name.to_str()).unwrap()
}

fn accent_themes() -> Vec<String> {
    let mut themes: Vec<String> = fs::read_dir(data().join("icons"))
        .unwrap()
        .filter_map(|entry| entry.unwrap().file_name().into_string().ok())
        .filter(|name| name.starts_with("Unbound-"))
        .collect();
    themes.sort();
    themes
}

fn load(theme: &str, icon: &str, pixels: u32, ink: &str) -> Image {
    let path = find_icon(icon, pixels, &[theme], &[data()])
        .unwrap_or_else(|| panic!("{icon} is missing from {theme} at {pixels} px"));
    let image = if icon.ends_with("-symbolic") {
        let svg = fs::read_to_string(&path)
            .unwrap()
            .replace(SYMBOLIC_INK, ink);
        Image::from_svg(svg.as_bytes(), pixels, pixels)
    } else {
        Image::load(&path, pixels)
    };
    image.unwrap_or_else(|| panic!("{} does not render at {pixels} px", path.display()))
}

fn contact_sheet(icons: &[(String, String)], background: Color, ink: &str) -> Scene {
    let cell_width = SIZES.iter().map(|size| *size as f32 + GAP).sum::<f32>() + GAP;
    let cell_height = 64.0 + 2.0 * GAP;
    let rows = icons.len().div_ceil(COLUMNS);
    let mut items = Vec::new();
    for (index, (theme, icon)) in icons.iter().enumerate() {
        let x = (index % COLUMNS) as f32 * cell_width + GAP;
        let y = (index / COLUMNS) as f32 * cell_height + GAP;
        let mut left = x;
        for size in SIZES {
            let pixels = size as f32;
            items.push(Item::Image {
                rect: Rect::new(left, y + 64.0 - pixels, pixels, pixels),
                image: load(theme, icon, size, ink),
            });
            left += pixels + GAP;
        }
    }
    Scene {
        width: (cell_width * COLUMNS as f32) as u32,
        height: (cell_height * rows as f32) as u32 + GAP as u32,
        background,
        items,
    }
}

fn check_golden(name: &str, scene: &Scene) {
    let frame = CpuRenderer::new().render(scene).unwrap();
    let actual = image::RgbaImage::from_raw(scene.width, scene.height, frame.rgba).unwrap();
    let actual = image::DynamicImage::ImageRgba8(actual).to_rgb8();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.png"));
    if std::env::var_os("UNBOUND_UPDATE_GOLDENS").is_some() {
        actual.save(&path).unwrap();
    }
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    actual.save(&target).unwrap();
    let expected = image::open(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .to_rgb8();
    let similarity = image_compare::rgb_hybrid_compare(&expected, &actual).unwrap();
    assert!(
        similarity.score >= MIN_SSIM,
        "{name} differs from its golden: {:.4} (frame in {})",
        similarity.score,
        target.display()
    );
}

fn every_icon() -> Vec<(String, String)> {
    let mut all: Vec<(String, String)> = icons(&theme())
        .iter()
        .map(|path| ("Unbound".to_owned(), name(path).to_owned()))
        .collect();
    all.extend(
        accent_themes()
            .into_iter()
            .map(|theme| (theme, "folder-download".to_owned())),
    );
    all
}

#[test]
fn the_theme_matches_its_light_contact_sheet() {
    let scene = contact_sheet(&every_icon(), Color::hex(0xFCFBF9), "#201E1A");
    check_golden("icons-light", &scene);
}

#[test]
fn the_theme_matches_its_dark_contact_sheet() {
    let scene = contact_sheet(&every_icon(), Color::hex(0x1D1B19), "#F1F0ED");
    check_golden("icons-dark", &scene);
}

#[test]
fn symbolic_icons_are_single_colour_fills_that_toolkits_can_recolour() {
    let symbolic: Vec<PathBuf> = icons(&theme())
        .into_iter()
        .filter(|path| name(path).ends_with("-symbolic"))
        .collect();
    assert!(symbolic.len() > 100);
    for path in symbolic {
        let svg = fs::read_to_string(&path).unwrap();
        assert!(!svg.contains("stroke"), "{} strokes", path.display());
        let fills: BTreeSet<&str> = svg
            .split("fill=\"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .collect();
        assert_eq!(
            fills,
            BTreeSet::from([SYMBOLIC_INK]),
            "{} paints other colours",
            path.display()
        );
    }
}

#[test]
fn every_app_we_ship_or_recommend_has_an_icon_in_the_theme() {
    let dirs = vec![data()];
    let mut names: Vec<String> =
        fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps"))
            .unwrap()
            .flat_map(|app| fs::read_dir(app.unwrap().path().join("data")).unwrap())
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "desktop")
            })
            .map(|path| {
                fs::read_to_string(&path)
                    .unwrap()
                    .lines()
                    .find_map(|line| line.strip_prefix("Icon="))
                    .unwrap()
                    .to_owned()
            })
            .collect();
    assert_eq!(names.len(), 4);
    names.extend(
        [
            "org.gnome.TextEditor",
            "org.gnome.Loupe",
            "org.gnome.Papers",
            "org.gnome.Showtime",
            "org.gnome.Calculator",
            "org.gnome.clocks",
        ]
        .map(str::to_owned),
    );
    for name in names {
        assert_eq!(
            find_icon(&name, 48, &ICON_THEMES, &dirs),
            Some(theme().join(format!("scalable/apps/{name}.svg"))),
            "{name}"
        );
    }
}
