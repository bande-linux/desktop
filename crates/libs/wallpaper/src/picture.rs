use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use image::imageops::{self, FilterType};
use image::{Rgba, RgbaImage};
use resvg::{tiny_skia, usvg};

use crate::choice::Fit;

const BACKDROP_REDUCTION: u32 = 10;
const BACKDROP_BLUR: f32 = 7.0;
const BACKDROP_BRIGHTNESS: f32 = 0.62;
const BACKDROP_SATURATION: f32 = 0.8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug)]
pub enum RenderError {
    Read(io::Error),
    Svg(usvg::Error),
    Decode(image::ImageError),
    Empty,
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(f, "cannot read the picture: {error}"),
            Self::Svg(error) => write!(f, "cannot read the drawing: {error}"),
            Self::Decode(error) => write!(f, "cannot decode the picture: {error}"),
            Self::Empty => write!(f, "the picture has no pixels"),
        }
    }
}

impl std::error::Error for RenderError {}

fn scale_for(fit: Fit, source: (f32, f32), target: (u32, u32)) -> f32 {
    let horizontal = target.0 as f32 / source.0;
    let vertical = target.1 as f32 / source.1;
    match fit {
        Fit::Fill => horizontal.max(vertical),
        Fit::Fit => horizontal.min(vertical),
    }
}

fn scaled_size(fit: Fit, source: (f32, f32), target: (u32, u32)) -> (u32, u32) {
    let scale = scale_for(fit, source, target);
    match fit {
        Fit::Fill => target,
        Fit::Fit => (
            ((source.0 * scale).round() as u32).clamp(1, target.0),
            ((source.1 * scale).round() as u32).clamp(1, target.1),
        ),
    }
}

fn noise(x: u32, y: u32) -> i16 {
    let mut hash = x.wrapping_mul(0x8DA6_B343) ^ y.wrapping_mul(0xD816_3841);
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(0x85EB_CA6B);
    hash ^= hash >> 16;
    i16::from((hash & 3) as u8) - i16::from((hash >> 2 & 3) as u8)
}

fn dither(image: &mut RgbaImage) {
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let offset = noise(x, y).signum();
        for channel in &mut pixel.0[..3] {
            *channel = (i16::from(*channel) + offset).clamp(0, 255) as u8;
        }
    }
}

fn drawing(bytes: &[u8], fit: Fit, target: (u32, u32)) -> Result<RgbaImage, RenderError> {
    let tree = usvg::Tree::from_data(bytes, &usvg::Options::default()).map_err(RenderError::Svg)?;
    let view = (tree.size().width(), tree.size().height());
    let scale = scale_for(fit, view, target);
    let (width, height) = scaled_size(fit, view, target);
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(RenderError::Empty)?;
    let offset_x = (width as f32 - view.0 * scale) / 2.0;
    let offset_y = (height as f32 - view.1 * scale) / 2.0;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, offset_x, offset_y),
        &mut pixmap.as_mut(),
    );
    let mut rgba = Vec::with_capacity(pixmap.data().len());
    for pixel in pixmap.pixels() {
        let color = pixel.demultiply();
        rgba.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
    }
    let mut image = RgbaImage::from_raw(width, height, rgba).ok_or(RenderError::Empty)?;
    dither(&mut image);
    Ok(image)
}

fn photo(bytes: &[u8], fit: Fit, target: (u32, u32)) -> Result<RgbaImage, RenderError> {
    let decoded = image::load_from_memory(bytes)
        .map_err(RenderError::Decode)?
        .to_rgba8();
    let source = (decoded.width() as f32, decoded.height() as f32);
    if decoded.width() == 0 || decoded.height() == 0 {
        return Err(RenderError::Empty);
    }
    let scale = scale_for(fit, source, target);
    let resized = imageops::resize(
        &decoded,
        ((source.0 * scale).ceil() as u32).max(1),
        ((source.1 * scale).ceil() as u32).max(1),
        FilterType::CatmullRom,
    );
    let (width, height) = scaled_size(fit, source, target);
    let (width, height) = (width.min(resized.width()), height.min(resized.height()));
    Ok(imageops::crop_imm(
        &resized,
        (resized.width() - width) / 2,
        (resized.height() - height) / 2,
        width,
        height,
    )
    .to_image())
}

fn edge_color(image: &RgbaImage) -> Rgba<u8> {
    let (width, height) = image.dimensions();
    let edge = (0..width)
        .flat_map(|x| [(x, 0), (x, height - 1)])
        .chain((0..height).flat_map(|y| [(0, y), (width - 1, y)]));
    let mut sum = [0u64; 3];
    let mut count = 0u64;
    for (x, y) in edge {
        let pixel = image.get_pixel(x, y);
        for (total, channel) in sum.iter_mut().zip(pixel.0) {
            *total += u64::from(channel);
        }
        count += 1;
    }
    let average = |total: u64| (total / count.max(1)) as u8;
    Rgba([average(sum[0]), average(sum[1]), average(sum[2]), 255])
}

fn framed(content: RgbaImage, target: (u32, u32)) -> RgbaImage {
    if content.dimensions() == target {
        return content;
    }
    let mut canvas = RgbaImage::from_pixel(target.0, target.1, edge_color(&content));
    imageops::overlay(
        &mut canvas,
        &content,
        i64::from((target.0 - content.width()) / 2),
        i64::from((target.1 - content.height()) / 2),
    );
    canvas
}

fn picture(image: RgbaImage) -> Picture {
    Picture {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    }
}

fn rendered(path: &Path, fit: Fit, width: u32, height: u32) -> Result<RgbaImage, RenderError> {
    let target = (width.max(1), height.max(1));
    let bytes = fs::read(path).map_err(RenderError::Read)?;
    let content = if path.extension().is_some_and(|extension| extension == "svg") {
        drawing(&bytes, fit, target)?
    } else {
        photo(&bytes, fit, target)?
    };
    Ok(framed(content, target))
}

pub fn render(path: &Path, fit: Fit, width: u32, height: u32) -> Result<Picture, RenderError> {
    rendered(path, fit, width, height).map(picture)
}

pub fn backdrop(path: &Path, fit: Fit, width: u32, height: u32) -> Result<Picture, RenderError> {
    let small = rendered(
        path,
        fit,
        width.div_ceil(BACKDROP_REDUCTION),
        height.div_ceil(BACKDROP_REDUCTION),
    )?;
    let mut blurred = imageops::fast_blur(&small, BACKDROP_BLUR);
    for pixel in blurred.pixels_mut() {
        let [red, green, blue, _] = pixel.0.map(f32::from);
        let luma = 0.2126 * red + 0.7152 * green + 0.0722 * blue;
        for channel in &mut pixel.0[..3] {
            let saturated = luma + (f32::from(*channel) - luma) * BACKDROP_SATURATION;
            *channel = (saturated * BACKDROP_BRIGHTNESS).round().clamp(0.0, 255.0) as u8;
        }
        pixel.0[3] = 255;
    }
    Ok(picture(blurred))
}
