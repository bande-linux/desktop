use std::fs::{self, File};
use std::io::Read;

use unbound_toolkit::Image;

use crate::listing::{Entry, Kind, list};

pub const TEXT_PREVIEW_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone)]
pub enum Preview {
    Image(Image),
    Text(String),
    Folder { items: usize },
    Other { size: u64 },
}

fn text(entry: &Entry) -> Option<String> {
    let mut bytes = Vec::new();
    File::open(&entry.path)
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

fn image(entry: &Entry) -> Option<Image> {
    let decoded = image::ImageReader::open(&entry.path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?
        .into_rgba8();
    let (width, height) = decoded.dimensions();
    Image::from_rgba(width, height, decoded.into_raw())
}

pub fn preview(entry: &Entry) -> Preview {
    if entry.kind == Kind::Folder {
        return Preview::Folder {
            items: list(&entry.path, false).map_or(0, |entries| entries.len()),
        };
    }
    if let Some(image) = image(entry) {
        return Preview::Image(image);
    }
    if let Some(text) = text(entry) {
        return Preview::Text(text);
    }
    Preview::Other {
        size: fs::metadata(&entry.path).map_or(0, |metadata| metadata.len()),
    }
}
