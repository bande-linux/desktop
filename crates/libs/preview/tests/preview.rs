#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use bande_fs::{Kind, Sort, list};
use bande_preview::{Preview, image_size, preview};
use tempfile::TempDir;

fn dimensions(width: u32, height: u32, _: Vec<u8>) -> Option<(u32, u32)> {
    Some((width, height))
}

#[test]
fn quick_look_previews_images_text_folders_and_other_files() {
    let root = TempDir::new().expect("temporary folder");
    let home = root.path();
    fs::create_dir_all(home.join("Documents/Reports")).expect("folders");
    fs::write(home.join("Documents/a.txt"), "first\n").expect("file");
    fs::write(home.join("notes.txt"), "notes\n").expect("file");
    image::RgbaImage::from_pixel(3, 2, image::Rgba([255, 0, 0, 255]))
        .save(home.join("picture.png"))
        .expect("png");
    fs::write(home.join("data.bin"), [0u8, 1, 2, 3, 4]).expect("binary");
    let entries = list(home, false, Sort::default()).expect("listing");
    let entry = |name: &str| {
        entries
            .iter()
            .find(|entry| entry.name == name)
            .expect("entry")
    };

    match preview(entry("picture.png"), dimensions) {
        Preview::Image(size) => assert_eq!(size, (3, 2)),
        other => panic!("{other:?}"),
    }
    match preview(entry("notes.txt"), dimensions) {
        Preview::Text(text) => assert_eq!(text, "notes\n"),
        other => panic!("{other:?}"),
    }
    assert_eq!(entry("Documents").kind, Kind::Folder);
    assert!(matches!(
        preview(entry("Documents"), dimensions),
        Preview::Folder { items: 2 }
    ));
    assert!(matches!(
        preview(entry("data.bin"), dimensions),
        Preview::Other { size: 5 }
    ));
}

#[test]
fn image_sizes_come_from_png_and_jpeg_headers() {
    let root = TempDir::new().expect("temporary folder");
    let png = root.path().join("wide.png");
    let jpeg = root.path().join("photo.jpg");
    image::RgbaImage::from_pixel(64, 20, image::Rgba([0, 0, 255, 255]))
        .save(&png)
        .expect("png");
    image::RgbImage::from_pixel(30, 48, image::Rgb([200, 10, 10]))
        .save(&jpeg)
        .expect("jpeg");
    fs::write(root.path().join("notes.txt"), "not an image").expect("text");

    assert_eq!(image_size(&png), Some((64, 20)));
    assert_eq!(image_size(&jpeg), Some((30, 48)));
    assert_eq!(image_size(&root.path().join("notes.txt")), None);
}

#[test]
fn a_fifo_is_previewed_as_an_other_file_without_waiting_for_a_writer() {
    let root = TempDir::new().expect("temporary folder");
    let fifo = root.path().join("pipe");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo");
    assert!(made.success());
    let entries = list(root.path(), false, Sort::default()).expect("listing");
    let entry = entries.into_iter().next().expect("the fifo");

    let (sender, previewed) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let other = matches!(preview(&entry, dimensions), Preview::Other { .. });
        sender.send(other)
    });
    assert_eq!(
        previewed.recv_timeout(std::time::Duration::from_secs(5)),
        Ok(true)
    );
}
