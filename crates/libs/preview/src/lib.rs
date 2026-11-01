use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use bande_fs::{Entry, is_hidden};

pub const TEXT_PREVIEW_BYTES: u64 = 64 * 1024;
pub const IMAGE_PREVIEW_PIXELS: u32 = 2048;
const DECODE_BYTES_LIMIT: u64 = 256 << 20;

#[derive(Debug, Clone)]
pub enum Preview<I> {
    Image(I),
    Text(String),
    Folder { items: usize },
    Other { size: u64 },
}

fn text(path: &Path) -> Option<String> {
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(TEXT_PREVIEW_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.contains(&0) {
        return None;
    }
    match String::from_utf8(bytes) {
        Ok(text) => Some(text),
        Err(error) if error.utf8_error().error_len().is_none() => {
            let valid = error.utf8_error().valid_up_to();
            let mut bytes = error.into_bytes();
            bytes.truncate(valid);
            String::from_utf8(bytes).ok()
        }
        Err(_) => None,
    }
}

fn image<I>(path: &Path, picture: impl FnOnce(u32, u32, Vec<u8>) -> Option<I>) -> Option<I> {
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(DECODE_BYTES_LIMIT);
    let mut reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    let decoded = if decoded.width().max(decoded.height()) > IMAGE_PREVIEW_PIXELS {
        decoded.thumbnail(IMAGE_PREVIEW_PIXELS, IMAGE_PREVIEW_PIXELS)
    } else {
        decoded
    };
    let rgba = decoded.into_rgba8();
    let (width, height) = rgba.dimensions();
    picture(width, height, rgba.into_raw())
}

pub fn image_size(path: &Path) -> Option<(u32, u32)> {
    image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

pub fn preview<I>(
    entry: &Entry,
    picture: impl FnOnce(u32, u32, Vec<u8>) -> Option<I>,
) -> Preview<I> {
    if entry.is_folder() {
        return Preview::Folder {
            items: fs::read_dir(&entry.path).map_or(0, |entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|entry| !is_hidden(&entry.file_name().to_string_lossy()))
                    .count()
            }),
        };
    }
    let metadata = fs::metadata(&entry.path);
    if !metadata.as_ref().is_ok_and(fs::Metadata::is_file) {
        return Preview::Other {
            size: metadata.map_or(0, |metadata| metadata.len()),
        };
    }
    if let Some(image) = image(&entry.path, picture) {
        return Preview::Image(image);
    }
    if let Some(text) = text(&entry.path) {
        return Preview::Text(text);
    }
    Preview::Other {
        size: fs::metadata(&entry.path).map_or(0, |metadata| metadata.len()),
    }
}
