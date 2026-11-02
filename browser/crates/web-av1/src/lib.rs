use std::ptr::NonNull;

use rav1d::include::dav1d::data::Dav1dData;
use rav1d::include::dav1d::dav1d::{Dav1dContext, Dav1dSettings};
use rav1d::include::dav1d::headers::{
    DAV1D_PIXEL_LAYOUT_I400, DAV1D_PIXEL_LAYOUT_I420, DAV1D_PIXEL_LAYOUT_I422,
};
use rav1d::include::dav1d::picture::Dav1dPicture;
use rav1d::src::lib::{
    dav1d_close, dav1d_data_create, dav1d_data_unref, dav1d_default_settings, dav1d_get_picture,
    dav1d_open, dav1d_picture_unref, dav1d_send_data,
};

const AGAIN: i32 = -11;

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

struct Plane<'a> {
    samples: &'a [u8],
    stride: usize,
}

struct Picture {
    picture: Dav1dPicture,
}

impl Drop for Picture {
    fn drop(&mut self) {
        if self.picture.r#ref.is_none() {
            return;
        }
        #[allow(unsafe_code)]
        // SAFETY: `picture` came from a successful `dav1d_get_picture` and is released once here.
        unsafe {
            dav1d_picture_unref(Some(NonNull::from(&mut self.picture)))
        };
    }
}

impl Picture {
    fn plane(&self, index: usize, rows: usize, row_bytes: usize) -> Option<Plane<'_>> {
        let data = self.picture.data.get(index).copied().flatten()?;
        let stride = usize::try_from(*self.picture.stride.get(index.min(1))?).ok()?;
        if stride < row_bytes || rows == 0 {
            return None;
        }
        let length = stride.checked_mul(rows - 1)?.checked_add(row_bytes)?;
        #[allow(unsafe_code)]
        // SAFETY: rav1d allocates every plane with `stride` bytes per row for all of its rows,
        // the picture stays referenced while `self` lives, and `length` stops at the last row's
        // visible bytes.
        let samples = unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), length) };
        Some(Plane { samples, stride })
    }
}

struct Decoder {
    context: Option<Dav1dContext>,
}

impl Drop for Decoder {
    fn drop(&mut self) {
        #[allow(unsafe_code)]
        // SAFETY: `context` is the one `dav1d_open` returned and is closed once here.
        unsafe {
            dav1d_close(Some(NonNull::from(&mut self.context)))
        };
    }
}

fn leb128(bytes: &[u8], at: &mut usize) -> Option<usize> {
    let mut value: u64 = 0;
    for index in 0..8 {
        let byte = *bytes.get(*at)?;
        *at += 1;
        value |= u64::from(byte & 0x7F) << (index * 7);
        if byte & 0x80 == 0 {
            return usize::try_from(value).ok();
        }
    }
    None
}

fn well_formed(obus: &[u8]) -> bool {
    const SEQUENCE_HEADER: u8 = 1;
    const FRAME_HEADER: u8 = 3;
    const TILE_GROUP: u8 = 4;
    const FRAME: u8 = 6;
    let mut at = 0;
    let (mut sequence, mut header) = (false, false);
    while at < obus.len() {
        let Some(&first) = obus.get(at) else {
            return false;
        };
        at += 1;
        let kind = (first >> 3) & 0xF;
        if first & 0x80 != 0 || first & 0x02 == 0 {
            return false;
        }
        if first & 0x04 != 0 {
            at += 1;
        }
        let Some(size) = leb128(obus, &mut at) else {
            return false;
        };
        let Some(end) = at.checked_add(size).filter(|&end| end <= obus.len()) else {
            return false;
        };
        match kind {
            SEQUENCE_HEADER => sequence = true,
            FRAME_HEADER | FRAME if !sequence => return false,
            FRAME_HEADER | FRAME => header = true,
            TILE_GROUP if !header => return false,
            _ => {}
        }
        at = end;
    }
    sequence && header
}

pub fn decode(obus: &[u8], max_pixels: u64, color: Option<(u16, bool)>) -> Option<Frame> {
    if !well_formed(obus) {
        return None;
    }
    let mut uninit = std::mem::MaybeUninit::<Dav1dSettings>::uninit();
    #[allow(unsafe_code)]
    // SAFETY: `dav1d_default_settings` writes a complete `Dav1dSettings` to the pointer, after
    // which every field is initialised.
    let mut settings = unsafe {
        dav1d_default_settings(NonNull::from(&mut uninit).cast());
        uninit.assume_init()
    };
    settings.n_threads = 1;
    settings.max_frame_delay = 1;
    settings.frame_size_limit = u32::try_from(max_pixels).unwrap_or(u32::MAX);
    let mut decoder = Decoder { context: None };
    #[allow(unsafe_code)]
    // SAFETY: both pointers refer to live locals of the right types.
    let opened = unsafe {
        dav1d_open(
            Some(NonNull::from(&mut decoder.context)),
            Some(NonNull::from(&mut settings)),
        )
    };
    if opened.0 != 0 || decoder.context.is_none() {
        return None;
    }
    let mut data = Dav1dData::default();
    #[allow(unsafe_code)]
    // SAFETY: `data` is a valid place for rav1d to describe its new buffer of `obus.len()` bytes.
    let buffer = unsafe { dav1d_data_create(Some(NonNull::from(&mut data)), obus.len()) };
    if buffer.is_null() {
        return None;
    }
    #[allow(unsafe_code)]
    // SAFETY: rav1d just allocated `buffer` with room for `obus.len()` bytes; the regions don't
    // overlap.
    unsafe {
        std::ptr::copy_nonoverlapping(obus.as_ptr(), buffer, obus.len())
    };
    let mut picture = Picture {
        picture: Dav1dPicture::default(),
    };
    let mut decoded = false;
    for _ in 0..64 {
        if data.sz > 0 {
            #[allow(unsafe_code)]
            // SAFETY: the context is open and `data` holds the remaining bytes of the buffer
            // created above.
            let sent = unsafe { dav1d_send_data(decoder.context, Some(NonNull::from(&mut data))) };
            if sent.0 != 0 && sent.0 != AGAIN {
                break;
            }
        }
        #[allow(unsafe_code)]
        // SAFETY: the context is open and `picture` is a valid, empty place for a frame.
        let got = unsafe {
            dav1d_get_picture(decoder.context, Some(NonNull::from(&mut picture.picture)))
        };
        if got.0 == 0 {
            decoded = true;
            break;
        }
        if got.0 != AGAIN {
            break;
        }
    }
    #[allow(unsafe_code)]
    // SAFETY: whatever rav1d did not consume of `data` is still ours to release.
    unsafe {
        dav1d_data_unref(Some(NonNull::from(&mut data)))
    };
    if !decoded {
        return None;
    }
    convert(&picture, max_pixels, color)
}

fn convert(picture: &Picture, max_pixels: u64, color: Option<(u16, bool)>) -> Option<Frame> {
    let parameters = &picture.picture.p;
    let width = usize::try_from(parameters.w).ok().filter(|&w| w > 0)?;
    let height = usize::try_from(parameters.h).ok().filter(|&h| h > 0)?;
    if (width as u64).checked_mul(height as u64)? > max_pixels {
        return None;
    }
    let bytes = if parameters.bpc > 8 { 2 } else { 1 };
    let shift = u32::try_from(parameters.bpc - 8).unwrap_or(0);
    let (sub_x, sub_y) = match parameters.layout {
        DAV1D_PIXEL_LAYOUT_I420 => (1, 1),
        DAV1D_PIXEL_LAYOUT_I422 => (1, 0),
        _ => (0, 0),
    };
    let mono = parameters.layout == DAV1D_PIXEL_LAYOUT_I400;
    let luma = picture.plane(0, height, width * bytes)?;
    let chroma_width = (width + sub_x) >> sub_x;
    let chroma_height = (height + sub_y) >> sub_y;
    let chroma = if mono {
        None
    } else {
        Some((
            picture.plane(1, chroma_height, chroma_width * bytes)?,
            picture.plane(2, chroma_height, chroma_width * bytes)?,
        ))
    };
    let header = picture.picture.seq_hdr.map(|header| {
        #[allow(unsafe_code)]
        // SAFETY: the sequence header is owned by the referenced picture and outlives this read.
        let header = unsafe { header.as_ref() };
        (header.mtrx as u16, header.color_range != 0)
    });
    let (matrix, full) = color.or(header).unwrap_or((1, false));
    let (kr, kb) = match matrix {
        1 => (0.2126, 0.0722),
        9 | 10 => (0.2627, 0.0593),
        _ => (0.299, 0.114),
    };
    let sample = |plane: &Plane<'_>, x: usize, y: usize| -> f32 {
        let at = y * plane.stride + x * bytes;
        let value = if bytes == 2 {
            plane
                .samples
                .get(at..at + 2)
                .map_or(0, |pair| u16::from_le_bytes([pair[0], pair[1]]))
        } else {
            plane.samples.get(at).copied().map_or(0, u16::from)
        };
        f32::from(value >> shift)
    };
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let mut luma_value = sample(&luma, x, y);
            if !full {
                luma_value = (luma_value - 16.0) * 255.0 / 219.0;
            }
            let (red, green, blue) = match &chroma {
                None => (luma_value, luma_value, luma_value),
                Some((u_plane, v_plane)) => {
                    let (cx, cy) = (x >> sub_x, y >> sub_y);
                    let (mut u, mut v) = (
                        sample(u_plane, cx, cy) - 128.0,
                        sample(v_plane, cx, cy) - 128.0,
                    );
                    if !full {
                        u *= 255.0 / 224.0;
                        v *= 255.0 / 224.0;
                    }
                    let red = luma_value + 2.0 * (1.0 - kr) * v;
                    let blue = luma_value + 2.0 * (1.0 - kb) * u;
                    let green = (luma_value - kr * red - kb * blue) / (1.0 - kr - kb);
                    (red, green, blue)
                }
            };
            let clamp = |value: f32| value.round().clamp(0.0, 255.0) as u8;
            rgba.extend_from_slice(&[clamp(red), clamp(green), clamp(blue), 255]);
        }
    }
    Some(Frame {
        width: width as u32,
        height: height as u32,
        rgba,
    })
}
