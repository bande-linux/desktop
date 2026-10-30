use std::path::PathBuf;

const MIN_SSIM: f64 = 0.99;
const WALLPAPER: u8 = 0x80;

pub fn check_golden(name: &str, size: (u32, u32), rgba: Vec<u8>) {
    let over_wallpaper = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| {
            let behind = u16::from(255 - pixel[3]) * u16::from(WALLPAPER) / 255;
            [0, 1, 2].map(|channel| (u16::from(pixel[channel]) + behind).min(255) as u8)
        })
        .collect();
    let actual = image::RgbImage::from_raw(size.0, size.1, over_wallpaper).expect("frame");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.png"));
    if std::env::var_os("UNBOUND_UPDATE_GOLDENS").is_some() {
        actual.save(&path).expect("write golden");
    }
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    actual.save(&target).expect("save the frame");
    let expected = image::open(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .to_rgb8();
    let similarity = image_compare::rgb_hybrid_compare(&expected, &actual).expect("same size");
    assert!(
        similarity.score >= MIN_SSIM,
        "{name} differs from its golden: {:.4} (frame in {})",
        similarity.score,
        target.display()
    );
}
