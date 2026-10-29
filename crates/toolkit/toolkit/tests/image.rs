#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use tempfile::TempDir;
use unbound_toolkit::Image;

const SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#ff0000"/></svg>"##;

#[test]
fn svg_images_are_rendered_at_the_requested_size() {
    let dir = TempDir::new().expect("dir");
    let path = dir.path().join("square.svg");
    fs::write(&path, SQUARE).expect("svg");

    let image = Image::load(&path, 32).expect("image");

    assert_eq!((image.width(), image.height()), (32, 32));
    assert!(
        image
            .premultiplied_rgba()
            .chunks(4)
            .all(|pixel| pixel == [255, 0, 0, 255])
    );
}

#[test]
fn an_unreadable_file_gives_no_image() {
    let dir = TempDir::new().expect("dir");

    assert!(Image::load(&dir.path().join("missing.png"), 32).is_none());
}
