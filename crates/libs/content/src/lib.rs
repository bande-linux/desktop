mod confined;
mod decode;
mod facts;
mod heif;
mod import;
pub mod pdf;
mod sandbox;
mod text;

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use bande_fs::Contents;
use bande_fs::{Entry, can_extract, is_hidden};

pub use confined::{WorkerFailure, decode_confined, run_worker, serve_worker};
pub use decode::{
    Decoded, Failure, Frame, Kind, Level, MAX_FILE_BYTES, MAX_PIXELS, Pixels, decode, open,
    read_file,
};
pub use facts::{Facts, camera_facts, container_facts};
pub use import::Importer;
pub use sandbox::only_compute_from_now_on;
pub use text::{Block, Code, Tone, highlight, html, is_code, markdown, rtf};

pub const TEXT_PREVIEW_BYTES: u64 = 64 * 1024;
pub const IMAGE_PREVIEW_PIXELS: u32 = 2048;
const PICTURE_SIDE: u32 = 4096;

#[derive(Debug, Clone)]
pub enum Preview<I> {
    Image(I),
    Text(String),
    Folder { items: usize },
    Other { size: u64 },
}

pub const TEXT_BYTES: u64 = 1 << 20;
const DOCUMENT_BYTES: usize = 256 << 10;
const LONGEST_LINE: usize = 4096;
const WRAPPED_BYTES: usize = 64 << 10;
const ADDRESS_SPACE: u64 = 6 << 30;
const WORKER_VARIABLE: &str = "BANDE_PREVIEW_WORKER";
const OWN_PROGRAM: &str = "/proc/self/exe";

pub enum Content {
    Picture(Decoded),
    Pdf(std::path::PathBuf),
    Code(Code),
    Document(Vec<Block>),
    Font,
    Folder { items: u64, bytes: Option<u64> },
    Archive,
    Media,
    Failed(Failure),
    Other,
}

fn text_up_to(path: &Path, limit: u64) -> Option<String> {
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(limit)
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

fn image<I>(
    path: &Path,
    picture: impl FnOnce(u32, u32, Vec<u8>) -> Option<I>,
) -> Option<(I, (u32, u32))> {
    let decoded = decode_confined(path, IMAGE_PREVIEW_PIXELS).ok()?;
    let size = decoded
        .facts
        .natural
        .unwrap_or((decoded.width, decoded.height));
    let level = match decoded.pixels {
        Pixels::Still(levels) => levels.into_iter().next()?,
        Pixels::Animation(frames) => Level {
            width: decoded.width,
            height: decoded.height,
            rgba: frames.into_iter().next()?.rgba,
        },
    };
    let rgba = Arc::try_unwrap(level.rgba).unwrap_or_else(|shared| (*shared).clone());
    Some((picture(level.width, level.height, rgba)?, size))
}

/// Returns the preview of `entry` and, for a picture, its size in pixels.
pub fn preview<I>(
    entry: &Entry,
    picture: impl FnOnce(u32, u32, Vec<u8>) -> Option<I>,
) -> (Preview<I>, Option<(u32, u32)>) {
    if entry.is_folder() {
        let items = fs::read_dir(&entry.path).map_or(0, |entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| !is_hidden(&entry.file_name().to_string_lossy()))
                .count()
        });
        return (Preview::Folder { items }, None);
    }
    let metadata = fs::metadata(&entry.path);
    if !metadata.as_ref().is_ok_and(fs::Metadata::is_file) {
        let size = metadata.map_or(0, |metadata| metadata.len());
        return (Preview::Other { size }, None);
    }
    if let Some((image, size)) = image(&entry.path, picture) {
        return (Preview::Image(image), Some(size));
    }
    if let Some(text) = text_up_to(&entry.path, TEXT_PREVIEW_BYTES) {
        return (Preview::Text(text), None);
    }
    let size = fs::metadata(&entry.path).map_or(0, |metadata| metadata.len());
    (Preview::Other { size }, None)
}

pub fn tree_size(path: &Path, current: &dyn Fn() -> bool) -> Option<(u64, u64)> {
    let mut items = 0;
    let mut bytes = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(folder) = pending.pop() {
        if !current() {
            return None;
        }
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            items += 1;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                bytes += metadata.len();
            }
        }
    }
    Some((items, bytes))
}

fn shown_items(folder: &Path) -> u64 {
    fs::read_dir(folder).map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .filter(|entry| !is_hidden(&entry.file_name().to_string_lossy()))
            .count() as u64
    })
}

fn is_font(mime: &str) -> bool {
    mime.starts_with("font/")
        || matches!(
            mime,
            "application/x-font-ttf"
                | "application/x-font-otf"
                | "application/vnd.ms-opentype"
                | "application/font-sfnt"
        )
}

fn is_text(mime: &str) -> bool {
    mime.starts_with("text/")
        || mime.ends_with("+xml")
        || mime.ends_with("+json")
        || matches!(
            mime,
            "application/json"
                | "application/xml"
                | "application/javascript"
                | "application/x-shellscript"
                | "application/toml"
                | "application/x-yaml"
                | "application/sql"
                | "application/x-desktop"
                | "application/rtf"
        )
}

pub fn content(path: &Path, mime: &str, current: &dyn Fn() -> bool) -> Content {
    let metadata = fs::metadata(path);
    if metadata.as_ref().is_ok_and(fs::Metadata::is_dir) {
        return Content::Folder {
            items: shown_items(path),
            bytes: tree_size(path, current).map(|(_, bytes)| bytes),
        };
    }
    if !metadata.as_ref().is_ok_and(fs::Metadata::is_file) {
        return Content::Other;
    }
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if mime == "application/pdf" {
        return Content::Pdf(path.to_path_buf());
    }
    let mut unshown = Content::Other;
    if mime.starts_with("image/") {
        match decode_confined(path, PICTURE_SIDE) {
            Ok(decoded) => return Content::Picture(decoded),
            Err(failure) => unshown = Content::Failed(failure),
        }
    }
    if is_font(mime) {
        return Content::Font;
    }
    if mime.starts_with("audio/") || mime.starts_with("video/") {
        return Content::Media;
    }
    if can_extract(path) {
        return Content::Archive;
    }
    let textual = is_text(mime) || is_code(&extension) || mime == "application/octet-stream";
    let Some(text) = textual.then(|| text_up_to(path, TEXT_BYTES)).flatten() else {
        return unshown;
    };
    let document = &text[..text.floor_char_boundary(DOCUMENT_BYTES)];
    match mime {
        "text/markdown" | "text/x-markdown" => Content::Document(markdown(document)),
        "text/rtf" | "application/rtf" => Content::Document(rtf(document)),
        "text/html" | "application/xhtml+xml" => Content::Document(html(document)),
        "text/plain" if !is_code(&extension) && text.len() <= WRAPPED_BYTES => {
            Content::Document(vec![Block::Plain(text)])
        }
        _ => Content::Code(highlight(broken_lines(text), &extension)),
    }
}

fn broken_lines(text: String) -> String {
    if text.lines().all(|line| line.len() <= LONGEST_LINE) {
        return text;
    }
    let mut broken = String::with_capacity(text.len() + text.len() / LONGEST_LINE);
    for line in text.split_inclusive('\n') {
        let mut rest = line;
        while rest.len() > LONGEST_LINE {
            let cut = rest.floor_char_boundary(LONGEST_LINE).max(1);
            let cut = rest.ceil_char_boundary(cut);
            broken.push_str(&rest[..cut]);
            broken.push('\n');
            rest = &rest[cut..];
        }
        broken.push_str(rest);
    }
    broken
}

pub fn worker_program() -> PathBuf {
    std::env::var_os(WORKER_VARIABLE)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(OWN_PROGRAM))
}

pub fn confine() {
    let limit = rustix::process::Rlimit {
        current: Some(ADDRESS_SPACE),
        maximum: Some(ADDRESS_SPACE),
    };
    if let Err(error) = rustix::process::setrlimit(rustix::process::Resource::As, limit) {
        tracing::warn!("cannot limit a file reader's memory: {error}");
    }
    if let Err(error) = rustix::thread::set_no_new_privs(true) {
        tracing::warn!("cannot keep a file reader from gaining privileges: {error}");
    }
    if let Err(error) =
        rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::KILL))
    {
        tracing::warn!("cannot tie a file reader to its viewer: {error}");
    }
}
