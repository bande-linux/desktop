use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use unbound_toolkit::Image;

use crate::listing::{Entry, list};

pub const TEXT_PREVIEW_BYTES: u64 = 64 * 1024;
pub const IMAGE_PREVIEW_PIXELS: u32 = 2048;

#[derive(Debug, Clone)]
pub enum Preview {
    Image(Image),
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

fn image(path: &Path) -> Option<Image> {
    let decoded = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let decoded = if decoded.width().max(decoded.height()) > IMAGE_PREVIEW_PIXELS {
        decoded.thumbnail(IMAGE_PREVIEW_PIXELS, IMAGE_PREVIEW_PIXELS)
    } else {
        decoded
    };
    let rgba = decoded.into_rgba8();
    let (width, height) = rgba.dimensions();
    Image::from_rgba(width, height, rgba.into_raw())
}

pub fn preview(entry: &Entry) -> Preview {
    if entry.is_folder() {
        return Preview::Folder {
            items: list(&entry.path, false).map_or(0, |entries| entries.len()),
        };
    }
    if let Some(image) = image(&entry.path) {
        return Preview::Image(image);
    }
    if let Some(text) = text(&entry.path) {
        return Preview::Text(text);
    }
    Preview::Other {
        size: fs::metadata(&entry.path).map_or(0, |metadata| metadata.len()),
    }
}
