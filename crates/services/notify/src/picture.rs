use std::collections::HashMap;

use zbus::zvariant::OwnedValue;

pub const MAX_PICTURE: u32 = 96;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picture {
    Pixels {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    File(String),
}

type ImageData = (i32, i32, i32, bool, i32, i32, Vec<u8>);

const PIXEL_HINTS: [&str; 2] = ["image-data", "image_data"];
const PATH_HINTS: [&str; 2] = ["image-path", "image_path"];
const LEGACY_PIXEL_HINT: &str = "icon_data";

pub(crate) fn picture_of(hints: &HashMap<String, OwnedValue>) -> Option<Picture> {
    let pixels = |name: &str| {
        hints
            .get(name)
            .and_then(|value| ImageData::try_from(value.try_clone().ok()?).ok())
            .and_then(pixels_of)
    };
    let path = |name: &str| {
        hints
            .get(name)
            .and_then(|value| <&str>::try_from(value).ok())
            .filter(|path| !path.is_empty())
            .map(|path| Picture::File(path.to_owned()))
    };
    PIXEL_HINTS
        .into_iter()
        .find_map(pixels)
        .or_else(|| PATH_HINTS.into_iter().find_map(path))
        .or_else(|| pixels(LEGACY_PIXEL_HINT))
}

fn pixels_of(
    (width, height, rowstride, has_alpha, bits_per_sample, channels, data): ImageData,
) -> Option<Picture> {
    let width = u32::try_from(width).ok().filter(|width| *width > 0)?;
    let height = u32::try_from(height).ok().filter(|height| *height > 0)?;
    let rowstride = usize::try_from(rowstride).ok()?;
    let channels = usize::try_from(channels).ok()?;
    if bits_per_sample != 8 || channels != if has_alpha { 4 } else { 3 } {
        return None;
    }
    let row = width as usize * channels;
    let needed = rowstride
        .checked_mul(height as usize - 1)?
        .checked_add(row)?;
    if rowstride < row || data.len() < needed {
        return None;
    }
    let scale = (width.max(height) as f32 / MAX_PICTURE as f32).max(1.0);
    let fitted = |side: u32| ((side as f32 / scale).round() as u32).max(1);
    let (out_width, out_height) = (fitted(width), fitted(height));
    let mut rgba = Vec::with_capacity((out_width * out_height * 4) as usize);
    for y in 0..out_height {
        let top = (y as f32 * scale) as u32;
        let bottom = (((y + 1) as f32 * scale) as u32).clamp(top + 1, height);
        for x in 0..out_width {
            let left = (x as f32 * scale) as u32;
            let right = (((x + 1) as f32 * scale) as u32).clamp(left + 1, width);
            let (mut colour, mut coverage) = ([0u32; 3], 0u32);
            for source_y in top..bottom {
                for source_x in left..right {
                    let at = source_y as usize * rowstride + source_x as usize * channels;
                    let alpha = if has_alpha {
                        u32::from(data[at + 3])
                    } else {
                        u32::from(u8::MAX)
                    };
                    for (channel, total) in colour.iter_mut().enumerate() {
                        *total += u32::from(data[at + channel]) * alpha;
                    }
                    coverage += alpha;
                }
            }
            let count = (bottom - top) * (right - left);
            rgba.extend(colour.map(|total| (total / coverage.max(1)) as u8));
            rgba.push((coverage / count) as u8);
        }
    }
    Some(Picture::Pixels {
        width: out_width,
        height: out_height,
        rgba,
    })
}
