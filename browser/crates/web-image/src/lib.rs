use std::io::Cursor;
use std::sync::Arc;

#[derive(PartialEq, Eq, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Box<[u8]>,
}

impl Image {
    pub fn bytes(&self) -> usize {
        self.rgba.len()
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
    Some(Image {
        width: output.width,
        height: output.height,
        rgba: rgba.into(),
    })
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
    (rgba.len() == width as usize * height as usize * 4).then(|| Image {
        width,
        height,
        rgba: rgba.into(),
    })
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
    Some(Image {
        width,
        height,
        rgba: rgba.into(),
    })
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
    Some(Image {
        width,
        height,
        rgba: canvas.into(),
    })
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
            .map(|image| (image.width as f32, image.height as f32))
    }

    pub fn bytes(&self) -> usize {
        self.entries
            .iter()
            .filter_map(|entry| entry.image.as_ref())
            .map(|image| image.bytes())
            .sum()
    }
}
