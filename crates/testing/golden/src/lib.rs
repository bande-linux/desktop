mod pseudo;
mod text;

use std::path::Path;

pub use image::RgbImage;
pub use pseudo::{review_pseudo, use_pseudo_locale};

const MIN_SIMILARITY: f64 = 0.99;

#[macro_export]
macro_rules! assert_golden {
    ($name:expr, $image:expr) => {
        $crate::check(
            env!("CARGO_MANIFEST_DIR"),
            env!("CARGO_TARGET_TMPDIR"),
            $name,
            $image,
        )
    };
}

pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> RgbImage {
    let Some(frame) = image::RgbaImage::from_raw(width, height, rgba) else {
        panic!(
            "a {width}x{height} frame needs {} bytes",
            width * height * 4
        );
    };
    image::DynamicImage::ImageRgba8(frame).to_rgb8()
}

pub fn check(manifest_dir: &str, scratch_dir: &str, name: &str, actual: &RgbImage) {
    let golden = Path::new(manifest_dir).join(format!("tests/golden/{name}.png"));
    if std::env::var_os("UNBOUND_UPDATE_GOLDENS").is_some()
        && let Err(error) = actual.save(&golden)
    {
        panic!("{}: {error}", golden.display());
    }
    let frame = Path::new(scratch_dir).join(format!("{name}.png"));
    if let Err(error) = actual.save(&frame) {
        panic!("{}: {error}", frame.display());
    }
    let expected = match image::open(&golden) {
        Ok(expected) => expected.to_rgb8(),
        Err(error) => panic!("{}: {error}", golden.display()),
    };
    let similarity = match image_compare::rgb_hybrid_compare(&expected, actual) {
        Ok(similarity) => similarity.score,
        Err(error) => panic!("{name}: {error} (frame in {})", frame.display()),
    };
    assert!(
        similarity >= MIN_SIMILARITY,
        "{name} differs from its golden: {similarity:.4} (frame in {})",
        frame.display()
    );
}
