use std::fs::OpenOptions;
use std::io::{Cursor, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::metadata::Orientation;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use jxl_oxide::{AllocTracker, JxlImage};
use libheif_rs::{ColorSpace, DecodingOptions, HeifContext, LibHeif, RgbChroma};
use serde::{Deserialize, Serialize};

use crate::facts::{Facts, camera_facts, container_facts};

pub const MAX_FILE_BYTES: u64 = 512 << 20;
pub const MAX_PIXELS: u64 = 100_000_000;
const DECODER_BYTES: u64 = 1 << 30;
const ANIMATION_BYTES: u64 = 192 << 20;
const MAX_FRAMES: usize = 4096;
const SMALLEST_LEVEL: u32 = 256;
const SVG_SIDE: f32 = 2048.0;
const SVG_MAX_SIDE: f32 = 8192.0;
const MIN_DELAY: Duration = Duration::from_millis(20);
const DEFAULT_DELAY: Duration = Duration::from_millis(100);
const JXL_CODESTREAM: &[u8] = &[0xff, 0x0a];
const JXL_CONTAINER: &[u8] = b"\0\0\0\x0cJXL \r\n\x87\n";
const FILE_TYPE_BOX: &[u8] = b"ftyp";
const AVIF_BRANDS: [&[u8; 4]; 2] = [b"avif", b"avis"];
const HEIF_BRANDS: [&[u8; 4]; 8] = [
    b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx", b"mif1", b"msf1",
];

pub struct Level {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

pub struct Frame {
    pub rgba: Arc<Vec<u8>>,
    pub delay: Duration,
}

pub enum Pixels {
    Still(Vec<Level>),
    Animation(Vec<Frame>),
}

pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
    pub facts: Facts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Failure {
    Unreadable,
    NotSupported,
    Damaged,
    TooLarge { width: u32, height: u32 },
    FileTooLarge,
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Png,
    Jpeg,
    Gif,
    Webp,
    Tiff,
    Bmp,
    Ico,
    Heif,
    Avif,
    Jxl,
    Svg,
    Pdf,
}

fn brands(bytes: &[u8]) -> Vec<&[u8; 4]> {
    if bytes.get(4..8) != Some(FILE_TYPE_BOX) {
        return Vec::new();
    }
    let size = bytes
        .get(..4)
        .and_then(|size| size.try_into().ok())
        .map_or(0, |size| u32::from_be_bytes(size) as usize);
    bytes
        .get(8..size.min(bytes.len()))
        .unwrap_or_default()
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != 1)
        .map(|(_, brand)| brand)
        .collect()
}

impl Kind {
    pub fn of(bytes: &[u8], path: &Path) -> Option<Self> {
        if bytes.starts_with(JXL_CODESTREAM) || bytes.starts_with(JXL_CONTAINER) {
            return Some(Self::Jxl);
        }
        let brands = brands(bytes);
        if brands.iter().any(|brand| AVIF_BRANDS.contains(brand)) {
            return Some(Self::Avif);
        }
        if brands.iter().any(|brand| HEIF_BRANDS.contains(brand)) {
            return Some(Self::Heif);
        }
        match image::guess_format(bytes) {
            Ok(ImageFormat::Png) => return Some(Self::Png),
            Ok(ImageFormat::Jpeg) => return Some(Self::Jpeg),
            Ok(ImageFormat::Gif) => return Some(Self::Gif),
            Ok(ImageFormat::WebP) => return Some(Self::Webp),
            Ok(ImageFormat::Tiff) => return Some(Self::Tiff),
            Ok(ImageFormat::Bmp) => return Some(Self::Bmp),
            Ok(ImageFormat::Ico) => return Some(Self::Ico),
            _ => {}
        }
        let head = &bytes[..bytes.len().min(1024)];
        if head.windows(5).any(|window| window == b"%PDF-") {
            return Some(Self::Pdf);
        }
        let svg_name = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"));
        let svg_text = head.windows(4).any(|window| window == b"<svg");
        (svg_name || svg_text).then_some(Self::Svg)
    }

    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Tiff => "image/tiff",
            Self::Bmp => "image/bmp",
            Self::Ico => "image/vnd.microsoft.icon",
            Self::Heif => "image/heif",
            Self::Avif => "image/avif",
            Self::Jxl => "image/jxl",
            Self::Svg => "image/svg+xml",
            Self::Pdf => "application/pdf",
        }
    }
}

pub fn read_file(path: &Path) -> Result<Vec<u8>, Failure> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
        .open(path)
        .map_err(|_| Failure::Unreadable)?;
    let metadata = file.metadata().map_err(|_| Failure::Unreadable)?;
    if !metadata.is_file() {
        return Err(Failure::Unreadable);
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(Failure::FileTooLarge);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::Unreadable)?;
    match bytes.len() as u64 > MAX_FILE_BYTES {
        true => Err(Failure::FileTooLarge),
        false => Ok(bytes),
    }
}

fn allowed(width: u32, height: u32) -> Result<(), Failure> {
    match width == 0 || height == 0 {
        true => Err(Failure::Damaged),
        false if u64::from(width) * u64::from(height) > MAX_PIXELS => {
            Err(Failure::TooLarge { width, height })
        }
        false => Ok(()),
    }
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_alloc = Some(DECODER_BYTES);
    limits
}

pub fn decode(bytes: &[u8], kind: Kind) -> Result<Decoded, Failure> {
    decode_with(bytes, kind, &svg_options(kind))
}

pub(crate) fn svg_options(kind: Kind) -> resvg::usvg::Options<'static> {
    let mut options = resvg::usvg::Options::default();
    if kind == Kind::Svg {
        options.fontdb_mut().load_system_fonts();
    }
    options
}

pub(crate) fn decode_with(
    bytes: &[u8],
    kind: Kind,
    svg: &resvg::usvg::Options<'_>,
) -> Result<Decoded, Failure> {
    match kind {
        Kind::Svg => decode_svg(bytes, svg),
        Kind::Gif => decode_gif(bytes),
        Kind::Webp => decode_webp(bytes),
        Kind::Png => decode_still(bytes, ImageFormat::Png),
        Kind::Jpeg => decode_still(bytes, ImageFormat::Jpeg),
        Kind::Tiff => decode_still(bytes, ImageFormat::Tiff),
        Kind::Bmp => decode_still(bytes, ImageFormat::Bmp),
        Kind::Ico => decode_still(bytes, ImageFormat::Ico),
        Kind::Heif | Kind::Avif => decode_heif(bytes),
        Kind::Jxl => decode_jxl(bytes),
        Kind::Pdf => Err(Failure::NotSupported),
    }
}

fn decode_still(bytes: &[u8], format: ImageFormat) -> Result<Decoded, Failure> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits());
    let mut decoder = reader.into_decoder().map_err(|_| Failure::Damaged)?;
    let (width, height) = decoder.dimensions();
    allowed(width, height)?;
    let exif = decoder.exif_metadata().ok().flatten();
    let orientation = exif
        .as_deref()
        .and_then(Orientation::from_exif_chunk)
        .unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(|error| match error {
        image::ImageError::Limits(_) => Failure::TooLarge { width, height },
        _ => Failure::Damaged,
    })?;
    image.apply_orientation(orientation);
    let facts = camera_facts(exif);
    Ok(still(image.into_rgba8(), facts))
}

fn decode_heif(bytes: &[u8]) -> Result<Decoded, Failure> {
    let context = HeifContext::read_from_bytes(bytes).map_err(|_| Failure::Damaged)?;
    let handle = context
        .primary_image_handle()
        .map_err(|_| Failure::Damaged)?;
    allowed(handle.width(), handle.height())?;
    let mut options = DecodingOptions::new().ok_or(Failure::Damaged)?;
    options.set_convert_hdr_to_8bit(true);
    let image = LibHeif::new()
        .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgba), Some(options))
        .map_err(|_| Failure::Damaged)?;
    let plane = image.planes().interleaved.ok_or(Failure::Damaged)?;
    allowed(plane.width, plane.height)?;
    let row = plane.width as usize * 4;
    let mut rgba = Vec::with_capacity(row * plane.height as usize);
    for line in plane
        .data
        .chunks(plane.stride.max(1))
        .take(plane.height as usize)
    {
        rgba.extend_from_slice(line.get(..row).ok_or(Failure::Damaged)?);
    }
    let picture =
        image::RgbaImage::from_raw(plane.width, plane.height, rgba).ok_or(Failure::Damaged)?;
    Ok(still(picture, container_facts(bytes)))
}

fn decode_jxl(bytes: &[u8]) -> Result<Decoded, Failure> {
    let image = JxlImage::builder()
        .alloc_tracker(AllocTracker::with_limit(DECODER_BYTES as usize))
        .read(bytes)
        .map_err(|_| Failure::Damaged)?;
    let (width, height) = (image.width(), image.height());
    allowed(width, height)?;
    let render = image.render_frame(0).map_err(|_| Failure::Damaged)?;
    let mut stream = render.stream();
    let channels = stream.channels() as usize;
    if (stream.width(), stream.height()) != (width, height) || channels == 0 {
        return Err(Failure::Damaged);
    }
    let mut samples = vec![0u8; width as usize * height as usize * channels];
    stream.write_to_buffer(&mut samples);
    let rgba = samples
        .chunks_exact(channels)
        .flat_map(|pixel| match *pixel {
            [grey] => [grey, grey, grey, u8::MAX],
            [grey, alpha] => [grey, grey, grey, alpha],
            [red, green, blue] => [red, green, blue, u8::MAX],
            [red, green, blue, alpha, ..] => [red, green, blue, alpha],
            [] => [0; 4],
        })
        .collect();
    let picture = image::RgbaImage::from_raw(width, height, rgba).ok_or(Failure::Damaged)?;
    Ok(still(picture, Facts::default()))
}

fn still(rgba: image::RgbaImage, facts: Facts) -> Decoded {
    let (width, height) = rgba.dimensions();
    Decoded {
        width,
        height,
        pixels: Pixels::Still(pyramid(width, height, rgba.into_raw())),
        facts,
    }
}

fn pyramid(width: u32, height: u32, rgba: Vec<u8>) -> Vec<Level> {
    let mut levels = vec![Level {
        width,
        height,
        rgba: Arc::new(rgba),
    }];
    while let Some(last) = levels.last()
        && last.width.max(last.height) > SMALLEST_LEVEL
    {
        let half = halve(last);
        levels.push(half);
    }
    levels
}

fn halve(level: &Level) -> Level {
    let (width, height) = (level.width.div_ceil(2), level.height.div_ceil(2));
    let stride = level.width as usize * 4;
    let last_x = level.width as usize - 1;
    let last_y = level.height as usize - 1;
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    let row_of = |y: usize| {
        level
            .rgba
            .get(y * stride..(y + 1) * stride)
            .unwrap_or_default()
    };
    for (y, row) in rgba.chunks_exact_mut(width as usize * 4).enumerate() {
        let (top, bottom) = (row_of((2 * y).min(last_y)), row_of((2 * y + 1).min(last_y)));
        for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (left, right) = (8 * x, 4 * (2 * x + 1).min(last_x));
            for (channel, value) in pixel.iter_mut().enumerate() {
                let at = |line: &[u8], start: usize| {
                    u32::from(line.get(start + channel).copied().unwrap_or(0))
                };
                let sum = at(top, left) + at(top, right) + at(bottom, left) + at(bottom, right);
                *value = ((sum + 2) / 4) as u8;
            }
        }
    }
    Level {
        width,
        height,
        rgba: Arc::new(rgba),
    }
}

fn animation<'a>(
    decoder: impl AnimationDecoder<'a>,
    width: u32,
    height: u32,
    kind: Kind,
) -> Result<Decoded, Failure> {
    let frame_bytes = u64::from(width) * u64::from(height) * 4;
    let fits = (ANIMATION_BYTES / frame_bytes.max(1)).max(1) as usize;
    let mut frames = Vec::new();
    let mut complete = true;
    for frame in decoder.into_frames() {
        if frames.len() >= fits.min(MAX_FRAMES) {
            complete = false;
            break;
        }
        let Ok(frame) = frame else {
            break;
        };
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = match denominator {
            0 => DEFAULT_DELAY,
            _ => Duration::from_millis(u64::from(numerator / denominator)),
        };
        let buffer = frame.into_buffer();
        if buffer.dimensions() != (width, height) {
            break;
        }
        frames.push(Frame {
            rgba: Arc::new(buffer.into_raw()),
            delay: match delay < MIN_DELAY {
                true => DEFAULT_DELAY,
                false => delay,
            },
        });
    }
    if frames.is_empty() {
        return Err(Failure::Damaged);
    }
    let facts = Facts::default().frames(frames.len(), complete);
    if frames.len() == 1 || !complete {
        let first = frames.swap_remove(0);
        let rgba = Arc::try_unwrap(first.rgba).unwrap_or_else(|shared| (*shared).clone());
        return Ok(Decoded {
            width,
            height,
            pixels: Pixels::Still(pyramid(width, height, rgba)),
            facts: Facts {
                kind: Some(kind),
                ..facts
            },
        });
    }
    Ok(Decoded {
        width,
        height,
        pixels: Pixels::Animation(frames),
        facts: Facts {
            kind: Some(kind),
            ..facts
        },
    })
}

fn decode_gif(bytes: &[u8]) -> Result<Decoded, Failure> {
    let mut decoder = GifDecoder::new(Cursor::new(bytes)).map_err(|_| Failure::Damaged)?;
    decoder.set_limits(limits()).map_err(|_| Failure::Damaged)?;
    let (width, height) = decoder.dimensions();
    allowed(width, height)?;
    animation(decoder, width, height, Kind::Gif)
}

fn decode_webp(bytes: &[u8]) -> Result<Decoded, Failure> {
    let mut decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(|_| Failure::Damaged)?;
    let (width, height) = decoder.dimensions();
    allowed(width, height)?;
    if decoder.has_animation() {
        return animation(decoder, width, height, Kind::Webp);
    }
    decoder.set_limits(limits()).map_err(|_| Failure::Damaged)?;
    let exif = decoder.exif_metadata().ok().flatten();
    let orientation = exif
        .as_deref()
        .and_then(Orientation::from_exif_chunk)
        .unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(|_| Failure::Damaged)?;
    image.apply_orientation(orientation);
    Ok(still(image.into_rgba8(), camera_facts(exif)))
}

fn decode_svg(bytes: &[u8], options: &resvg::usvg::Options<'_>) -> Result<Decoded, Failure> {
    let tree = resvg::usvg::Tree::from_data(bytes, options).map_err(|_| Failure::Damaged)?;
    let size = tree.size();
    let longest = size.width().max(size.height());
    if !longest.is_finite() || longest <= 0.0 {
        return Err(Failure::Damaged);
    }
    let scale = (SVG_SIDE / longest).max(1.0).min(SVG_MAX_SIDE / longest);
    let width = (size.width() * scale).round().max(1.0) as u32;
    let height = (size.height() * scale).round().max(1.0) as u32;
    allowed(width, height)?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or(Failure::Damaged)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    let facts = Facts {
        kind: Some(Kind::Svg),
        natural: Some((size.width().round() as u32, size.height().round() as u32)),
        ..Facts::default()
    };
    Ok(Decoded {
        width,
        height,
        pixels: Pixels::Still(pyramid(width, height, rgba)),
        facts,
    })
}

pub fn open(path: &Path) -> Result<(Kind, Vec<u8>), Failure> {
    let bytes = read_file(path)?;
    let kind = Kind::of(&bytes, path).ok_or(Failure::NotSupported)?;
    Ok((kind, bytes))
}
