use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, BufReader, BufWriter};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use md5::{Digest, Md5};
use png::{BitDepth, ColorType, Transformations};

const SOFTWARE: &str = concat!("unbound-thumbnails-", env!("CARGO_PKG_VERSION"));
const URI_KEY: &str = "Thumb::URI";
const MTIME_KEY: &str = "Thumb::MTime";
const SOFTWARE_KEY: &str = "Software";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ThumbnailSize {
    Normal,
    Large,
    XLarge,
    XxLarge,
}

impl ThumbnailSize {
    const ALL: [Self; 4] = [Self::Normal, Self::Large, Self::XLarge, Self::XxLarge];

    pub const fn pixels(self) -> u32 {
        match self {
            Self::Normal => 128,
            Self::Large => 256,
            Self::XLarge => 512,
            Self::XxLarge => 1024,
        }
    }

    fn folder(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Large => "large",
            Self::XLarge => "x-large",
            Self::XxLarge => "xx-large",
        }
    }

    pub fn for_pixels(pixels: u32) -> Self {
        Self::ALL
            .into_iter()
            .find(|size| size.pixels() >= pixels)
            .unwrap_or(Self::XxLarge)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn kept_in_uri(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-_.!~*'()/:@&=+$,".contains(&byte)
}

pub fn uri(path: &Path) -> String {
    let mut encoded = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if kept_in_uri(byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

pub fn modified(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|age| age.as_secs())
}

fn digest(uri: &str) -> String {
    Md5::digest(uri.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cache {
    root: PathBuf,
}

struct Stored {
    uri: Option<String>,
    mtime: Option<u64>,
    picture: Option<Picture>,
}

fn rgba(color: ColorType, pixels: Vec<u8>) -> Vec<u8> {
    match color {
        ColorType::Rgba => pixels,
        ColorType::Rgb => pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|[red, green, blue]| [*red, *green, *blue, 255])
            .collect(),
        ColorType::GrayscaleAlpha => pixels
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|[gray, alpha]| [*gray, *gray, *gray, *alpha])
            .collect(),
        ColorType::Grayscale | ColorType::Indexed => pixels
            .into_iter()
            .flat_map(|gray| [gray, gray, gray, 255])
            .collect(),
    }
}

fn read(path: &Path, with_pixels: bool) -> Option<Stored> {
    let mut decoder = png::Decoder::new(BufReader::new(File::open(path).ok()?));
    decoder.set_transformations(Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let text = |key: &str| {
        reader
            .info()
            .uncompressed_latin1_text
            .iter()
            .find(|chunk| chunk.keyword == key)
            .map(|chunk| chunk.text.clone())
    };
    let uri = text(URI_KEY);
    let mtime = text(MTIME_KEY).and_then(|mtime| mtime.trim().parse().ok());
    let picture = if with_pixels {
        let mut buffer = vec![0; reader.output_buffer_size()?];
        let frame = reader.next_frame(&mut buffer).ok()?;
        buffer.truncate(frame.buffer_size());
        Some(Picture {
            width: frame.width,
            height: frame.height,
            rgba: rgba(frame.color_type, buffer),
        })
    } else {
        None
    };
    Some(Stored {
        uri,
        mtime,
        picture,
    })
}

fn write(path: &Path, uri: &str, mtime: u64, picture: &Picture) -> io::Result<()> {
    let folder = path.parent().ok_or(io::ErrorKind::NotFound)?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(folder)?;
    let staging = folder.join(format!(
        ".{}.{}.partial",
        path.file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default(),
        std::process::id()
    ));
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&staging)?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), picture.width, picture.height);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let invalid = |error: png::EncodingError| io::Error::new(io::ErrorKind::InvalidData, error);
    for (key, value) in [
        (URI_KEY, uri.to_owned()),
        (MTIME_KEY, mtime.to_string()),
        (SOFTWARE_KEY, SOFTWARE.to_owned()),
    ] {
        encoder
            .add_text_chunk(key.to_owned(), value)
            .map_err(invalid)?;
    }
    let written = encoder
        .write_header()
        .and_then(|mut writer| {
            writer.write_image_data(&picture.rgba)?;
            writer.finish()
        })
        .map_err(invalid);
    match written {
        Ok(()) => fs::rename(&staging, path),
        Err(error) => {
            let _ = fs::remove_file(&staging);
            Err(error)
        }
    }
}

impl Cache {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn for_user() -> Option<Self> {
        unbound_xdg::cache_home().map(|cache| Self::new(cache.join("thumbnails")))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self, uri: &str, size: ThumbnailSize) -> PathBuf {
        self.root
            .join(size.folder())
            .join(format!("{}.png", digest(uri)))
    }

    fn failure_path(&self, uri: &str) -> PathBuf {
        self.root
            .join("fail")
            .join(SOFTWARE)
            .join(format!("{}.png", digest(uri)))
    }

    pub fn load(&self, uri: &str, mtime: u64, size: ThumbnailSize) -> Option<Picture> {
        let stored = read(&self.path(uri, size), true)?;
        (stored.uri.as_deref() == Some(uri) && stored.mtime == Some(mtime))
            .then_some(stored.picture)
            .flatten()
    }

    pub fn has_failed(&self, uri: &str, mtime: u64) -> bool {
        read(&self.failure_path(uri), false)
            .is_some_and(|stored| stored.uri.as_deref() == Some(uri) && stored.mtime == Some(mtime))
    }

    pub fn store(
        &self,
        uri: &str,
        mtime: u64,
        size: ThumbnailSize,
        picture: &Picture,
    ) -> io::Result<()> {
        write(&self.path(uri, size), uri, mtime, picture)
    }

    pub fn record_failure(&self, uri: &str, mtime: u64) -> io::Result<()> {
        let blank = Picture {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        };
        write(&self.failure_path(uri), uri, mtime, &blank)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_md5_of_the_uri_as_in_the_spec_example() {
        let cache = Cache::new(PathBuf::from("/cache"));
        assert_eq!(
            cache.path("file:///home/jens/photos/me.png", ThumbnailSize::Normal),
            PathBuf::from("/cache/normal/c6ee772d9e49320e97ec29a7eb5b1697.png")
        );
    }

    #[test]
    fn uris_escape_like_glib_so_other_desktops_share_the_cache() {
        assert_eq!(
            uri(Path::new("/home/me/Résumé (final) #2&a=b.pdf")),
            "file:///home/me/R%C3%A9sum%C3%A9%20(final)%20%232&a=b.pdf"
        );
    }

    #[test]
    fn sizes_cover_the_wanted_pixels() {
        assert_eq!(ThumbnailSize::for_pixels(64), ThumbnailSize::Normal);
        assert_eq!(ThumbnailSize::for_pixels(129), ThumbnailSize::Large);
        assert_eq!(ThumbnailSize::for_pixels(400), ThumbnailSize::XLarge);
        assert_eq!(ThumbnailSize::for_pixels(4000), ThumbnailSize::XxLarge);
    }
}
