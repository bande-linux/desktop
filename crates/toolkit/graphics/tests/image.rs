#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use tempfile::TempDir;
use unbound_graphics::{Color, Image};

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

#[test]
fn a_symbolic_icon_takes_the_colour_it_is_tinted_with_and_keeps_its_shape() {
    let symbol = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="8" height="16" fill="#201E1A"/></svg>"##;
    let image = Image::from_svg(symbol.as_bytes(), 16, 16).expect("image");

    let tinted = image.tinted(Color::hex(0x4859DD)).expect("tinted");

    for (row, pixel) in tinted.premultiplied_rgba().chunks(4).enumerate() {
        let expected: [u8; 4] = if row % 16 < 8 {
            [0x48, 0x59, 0xDD, 0xFF]
        } else {
            [0, 0, 0, 0]
        };
        assert_eq!(pixel, expected, "pixel {row}");
    }
}
