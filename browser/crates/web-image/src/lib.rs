#[cfg(feature = "avif")]
mod avif;

use std::io::Cursor;
use std::sync::Arc;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub natural: (u32, u32),
    pub premultiplied: Arc<[u8]>,
}

impl Image {
    fn new(width: u32, height: u32, mut rgba: Vec<u8>) -> Self {
        for pixel in rgba.as_chunks_mut::<4>().0 {
            let alpha = u16::from(pixel[3]);
            if alpha != 255 {
                for channel in &mut pixel[..3] {
                    *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
                }
            }
        }
        Image {
            width,
            height,
            natural: (width, height),
            premultiplied: rgba.into(),
        }
    }

    fn scaled(&self, factor: u32) -> Image {
        let width = (self.width / factor).max(1);
        let height = (self.height / factor).max(1);
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        let source = &self.premultiplied;
        let stride = self.width as usize * 4;
        let count = factor * factor;
        for y in 0..height as usize {
            for x in 0..width as usize {
                let mut sum = [0u32; 4];
                for dy in 0..factor as usize {
                    let row = (y * factor as usize + dy) * stride;
                    for dx in 0..factor as usize {
                        let at = row + (x * factor as usize + dx) * 4;
                        if let Some(pixel) = source.get(at..at + 4) {
                            for (total, channel) in sum.iter_mut().zip(pixel) {
                                *total += u32::from(*channel);
                            }
                        }
                    }
                }
                let at = (y * width as usize + x) * 4;
                if let Some(target) = pixels.get_mut(at..at + 4) {
                    for (channel, total) in target.iter_mut().zip(sum) {
                        *channel = (total / count) as u8;
                    }
                }
            }
        }
        Image {
            width,
            height,
            natural: self.natural,
            premultiplied: pixels.into(),
        }
    }

    pub fn bytes(&self) -> usize {
        self.premultiplied.len()
    }
}

const MAX_SIDE: u32 = 16384;
const MAX_PIXELS: u64 = 64 << 20;

fn allowed(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= MAX_SIDE
        && height <= MAX_SIDE
        && u64::from(width) * u64::from(height) <= MAX_PIXELS
}

pub fn decode(bytes: &[u8]) -> Option<Image> {
    let image = match bytes {
        [0x89, b'P', b'N', b'G', ..] => decode_png(bytes),
        [0xFF, 0xD8, 0xFF, ..] => decode_jpeg(bytes),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => decode_webp(bytes),
        [b'G', b'I', b'F', b'8', ..] => decode_gif(bytes),
        [0xFF, 0x0A, ..] | [0, 0, 0, 0x0C, b'J', b'X', b'L', b' ', ..] => decode_jxl(bytes),
        #[cfg(feature = "avif")]
        [
            _,
            _,
            _,
            _,
            b'f',
            b't',
            b'y',
            b'p',
            b'a',
            b'v',
            b'i',
            b'f' | b's',
            ..,
        ] => decode_avif(bytes),
        _ => None,
    }?;
    allowed(image.width, image.height).then_some(image)
}

fn decode_png(bytes: &[u8]) -> Option<Image> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    if !allowed(info.width, info.height) {
        return None;
    }
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let output = reader.next_frame(&mut buffer).ok()?;
    let pixels = buffer.get(..output.buffer_size())?;
    let rgba: Vec<u8> = match output.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => pixels
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|&[gray, alpha]| [gray, gray, gray, alpha])
            .collect(),
        png::ColorType::Grayscale => pixels
            .iter()
            .flat_map(|&gray| [gray, gray, gray, 255])
            .collect(),
        png::ColorType::Indexed => return None,
    };
    Some(Image::new(output.width, output.height, rgba))
}

fn decode_jpeg(bytes: &[u8]) -> Option<Image> {
    let options = zune_core::options::DecoderOptions::default()
        .jpeg_set_out_colorspace(zune_core::colorspace::ColorSpace::RGBA)
        .set_max_width(MAX_SIDE as usize)
        .set_max_height(MAX_SIDE as usize);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(
        zune_core::bytestream::ZCursor::new(bytes),
        options,
    );
    decoder.decode_headers().ok()?;
    let info = decoder.info()?;
    let (width, height) = (u32::from(info.width), u32::from(info.height));
    if !allowed(width, height) {
        return None;
    }
    let rgba = decoder.decode().ok()?;
    (rgba.len() == width as usize * height as usize * 4).then(|| Image::new(width, height, rgba))
}

fn decode_webp(bytes: &[u8]) -> Option<Image> {
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
    let (width, height) = decoder.dimensions();
    if !allowed(width, height) {
        return None;
    }
    let mut buffer = vec![0; decoder.output_buffer_size()?];
    decoder.read_image(&mut buffer).ok()?;
    let rgba = if decoder.has_alpha() {
        buffer
    } else {
        buffer
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, 255])
            .collect()
    };
    Some(Image::new(width, height, rgba))
}

#[cfg(feature = "avif")]
fn decode_avif(bytes: &[u8]) -> Option<Image> {
    let (obus, color) = avif::obus(bytes)?;
    let frame = web_av1::decode(&obus, MAX_PIXELS, color)?;
    allowed(frame.width, frame.height).then(|| Image::new(frame.width, frame.height, frame.rgba))
}

fn decode_jxl(bytes: &[u8]) -> Option<Image> {
    let image = jxl_oxide::JxlImage::builder().read(bytes).ok()?;
    if !allowed(image.width(), image.height()) {
        return None;
    }
    let render = image.render_frame(0).ok()?;
    let mut stream = render.stream();
    let (width, height, channels) = (stream.width(), stream.height(), stream.channels());
    if !allowed(width, height) || channels == 0 {
        return None;
    }
    let size = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(channels as usize)?;
    let mut samples = vec![0u8; size];
    stream.write_to_buffer(&mut samples);
    let rgba = samples
        .chunks_exact(channels as usize)
        .flat_map(|pixel| match *pixel {
            [gray] => [gray, gray, gray, u8::MAX],
            [gray, alpha] => [gray, gray, gray, alpha],
            [red, green, blue] => [red, green, blue, u8::MAX],
            [red, green, blue, alpha, ..] => [red, green, blue, alpha],
            [] => [0; 4],
        })
        .collect();
    Some(Image::new(width, height, rgba))
}

fn decode_gif(bytes: &[u8]) -> Option<Image> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(Cursor::new(bytes)).ok()?;
    let (width, height) = (u32::from(decoder.width()), u32::from(decoder.height()));
    if !allowed(width, height) {
        return None;
    }
    let mut canvas = vec![0u8; width as usize * height as usize * 4];
    let frame = decoder.read_next_frame().ok()??;
    let (left, top) = (usize::from(frame.left), usize::from(frame.top));
    let frame_width = usize::from(frame.width);
    if frame_width == 0 {
        return None;
    }
    for (row, line) in frame.buffer.chunks_exact(frame_width * 4).enumerate() {
        let y = top + row;
        if y >= height as usize {
            break;
        }
        let start = (y * width as usize + left) * 4;
        let available = (width as usize).saturating_sub(left) * 4;
        let length = line.len().min(available);
        if let (Some(target), Some(source)) =
            (canvas.get_mut(start..start + length), line.get(..length))
        {
            target.copy_from_slice(source);
        }
    }
    Some(Image::new(width, height, canvas))
}

struct Entry {
    url: Box<str>,
    image: Option<Arc<Image>>,
    used: u64,
}

pub struct ImageCache {
    entries: Vec<Entry>,
    budget: usize,
    clock: u64,
}

impl ImageCache {
    pub fn new(budget: usize) -> Self {
        ImageCache {
            entries: Vec::new(),
            budget,
            clock: 0,
        }
    }

    pub fn contains(&self, url: &str) -> bool {
        self.entries.iter().any(|entry| &*entry.url == url)
    }

    pub fn insert(&mut self, url: &str, bytes: &[u8]) {
        let image = decode(bytes).map(Arc::new);
        self.clock += 1;
        let used = self.clock;
        match self.entries.iter_mut().find(|entry| &*entry.url == url) {
            Some(entry) => {
                entry.image = image;
                entry.used = used;
            }
            None => self.entries.push(Entry {
                url: url.into(),
                image,
                used,
            }),
        }
        self.evict();
    }

    fn evict(&mut self) {
        while self.bytes() > self.budget {
            let oldest = self
                .entries
                .iter_mut()
                .filter(|entry| entry.image.is_some())
                .min_by_key(|entry| entry.used);
            match oldest {
                Some(entry) => entry.image = None,
                None => break,
            }
        }
    }

    pub fn get(&self, url: &str) -> Option<&Arc<Image>> {
        self.entries
            .iter()
            .find(|entry| &*entry.url == url)
            .and_then(|entry| entry.image.as_ref())
    }

    pub fn size(&self, url: &str) -> Option<(f32, f32)> {
        self.get(url)
            .map(|image| (image.natural.0 as f32, image.natural.1 as f32))
    }

    pub fn fit(&mut self, url: &str, width: f32, height: f32) {
        let Some(entry) = self.entries.iter_mut().find(|entry| &*entry.url == url) else {
            return;
        };
        let Some(image) = &entry.image else {
            return;
        };
        let ratio =
            (image.width as f32 / width.max(1.0)).min(image.height as f32 / height.max(1.0));
        let factor = (ratio.floor() as u32).min(64);
        if factor >= 2 {
            entry.image = Some(Arc::new(image.scaled(factor)));
        }
    }

    pub fn bytes(&self) -> usize {
        self.entries
            .iter()
            .filter_map(|entry| entry.image.as_ref())
            .map(|image| image.bytes())
            .sum()
    }
}
