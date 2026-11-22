use flate2::{Decompress, FlushDecompress, Status};

use crate::encode::Frame;
use crate::protocol::{Area, ENCODING_RAW, ENCODING_ZRLE, Rectangle};

const TILE: usize = 64;
const BYTES_PER_PIXEL: usize = 4;
const COMPACT_PIXEL: usize = 3;
const MOST_INFLATED: usize = TILE * TILE * BYTES_PER_PIXEL + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Corrupt;

/// Decodes what a server sent in 32-bit little-endian true colour (`PixelFormat::NATIVE`).
pub struct Decoder {
    zlib: Decompress,
}

impl Default for Decoder {
    fn default() -> Self {
        Self {
            zlib: Decompress::new(true),
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], Corrupt> {
        let taken = self.bytes.get(self.at..self.at + count).ok_or(Corrupt)?;
        self.at += count;
        Ok(taken)
    }

    fn byte(&mut self) -> Result<u8, Corrupt> {
        Ok(self.take(1)?[0])
    }

    fn pixel(&mut self) -> Result<u32, Corrupt> {
        let bytes = self.take(COMPACT_PIXEL)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0]))
    }

    fn run(&mut self) -> Result<usize, Corrupt> {
        let mut length = 1;
        loop {
            let byte = self.byte()?;
            length += usize::from(byte);
            if byte != 255 {
                return Ok(length);
            }
        }
    }
}

impl Decoder {
    fn inflate(&mut self, input: &[u8], limit: usize) -> Result<Vec<u8>, Corrupt> {
        let mut output = Vec::with_capacity(input.len().saturating_mul(4).min(limit));
        let start = self.zlib.total_in();
        loop {
            let consumed = (self.zlib.total_in() - start) as usize;
            if output.len() == output.capacity() {
                if output.len() >= limit {
                    return Err(Corrupt);
                }
                output.reserve(output.capacity().max(4096).min(limit - output.len()));
            }
            let produced = output.len();
            let status = self
                .zlib
                .decompress_vec(&input[consumed..], &mut output, FlushDecompress::Sync)
                .map_err(|_| Corrupt)?;
            let consumed = (self.zlib.total_in() - start) as usize;
            let stalled = output.len() == produced;
            if consumed == input.len() && (output.len() < output.capacity() || stalled) {
                return Ok(output);
            }
            if status == Status::StreamEnd || (stalled && status == Status::BufError) {
                return Err(Corrupt);
            }
        }
    }

    /// Paints `rectangle` into `frame`; the area was checked to lie inside the screen.
    pub fn paint(&mut self, frame: &mut Frame, rectangle: &Rectangle) -> Result<(), Corrupt> {
        let Area {
            x,
            y,
            width,
            height,
        } = rectangle.area;
        let (x, y, width, height) = (
            usize::from(x),
            usize::from(y),
            usize::from(width),
            usize::from(height),
        );
        if x + width > frame.width as usize || y + height > frame.height as usize {
            return Err(Corrupt);
        }
        let stride = frame.width as usize;
        match rectangle.encoding {
            ENCODING_RAW => {
                let pixels = rectangle.data.as_chunks::<4>().0;
                if pixels.len() != width * height {
                    return Err(Corrupt);
                }
                for (row, line) in pixels.chunks(width.max(1)).enumerate() {
                    let start = (y + row) * stride + x;
                    for (target, pixel) in frame.pixels[start..start + width].iter_mut().zip(line) {
                        *target = u32::from_le_bytes(*pixel) & 0xff_ffff;
                    }
                }
                Ok(())
            }
            ENCODING_ZRLE => {
                let limit = MOST_INFLATED * width.div_ceil(TILE) * height.div_ceil(TILE);
                let inflated = self.inflate(&rectangle.data, limit)?;
                let mut reader = Reader {
                    bytes: &inflated,
                    at: 0,
                };
                let mut tile = Vec::with_capacity(TILE * TILE);
                for tile_y in (y..y + height).step_by(TILE) {
                    let tile_height = TILE.min(y + height - tile_y);
                    for tile_x in (x..x + width).step_by(TILE) {
                        let tile_width = TILE.min(x + width - tile_x);
                        tile.clear();
                        read_tile(&mut reader, tile_width, tile_height, &mut tile)?;
                        for row in 0..tile_height {
                            let start = (tile_y + row) * stride + tile_x;
                            frame.pixels[start..start + tile_width]
                                .copy_from_slice(&tile[row * tile_width..(row + 1) * tile_width]);
                        }
                    }
                }
                Ok(())
            }
            _ => Err(Corrupt),
        }
    }
}

fn read_tile(
    reader: &mut Reader<'_>,
    width: usize,
    height: usize,
    tile: &mut Vec<u32>,
) -> Result<(), Corrupt> {
    let count = width * height;
    let kind = reader.byte()?;
    match kind {
        0 => {
            for _ in 0..count {
                tile.push(reader.pixel()?);
            }
        }
        1 => {
            let pixel = reader.pixel()?;
            tile.resize(count, pixel);
        }
        2..=16 => {
            let palette: Vec<u32> = (0..kind)
                .map(|_| reader.pixel())
                .collect::<Result<_, _>>()?;
            let bits = match kind {
                2 => 1,
                3..=4 => 2,
                _ => 4,
            };
            for _ in 0..height {
                let row = reader.take((width * bits).div_ceil(8))?;
                for column in 0..width {
                    let bit = column * bits;
                    let index = (row[bit / 8] >> (8 - bits - bit % 8)) & ((1 << bits) - 1);
                    tile.push(*palette.get(usize::from(index)).ok_or(Corrupt)?);
                }
            }
        }
        128 => {
            while tile.len() < count {
                let pixel = reader.pixel()?;
                let run = reader.run()?;
                if tile.len() + run > count {
                    return Err(Corrupt);
                }
                tile.resize(tile.len() + run, pixel);
            }
        }
        130..=255 => {
            let palette: Vec<u32> = (0..kind - 128)
                .map(|_| reader.pixel())
                .collect::<Result<_, _>>()?;
            while tile.len() < count {
                let index = reader.byte()?;
                let pixel = *palette.get(usize::from(index & 0x7f)).ok_or(Corrupt)?;
                let run = match index & 0x80 {
                    0 => 1,
                    _ => reader.run()?,
                };
                if tile.len() + run > count {
                    return Err(Corrupt);
                }
                tile.resize(tile.len() + run, pixel);
            }
        }
        _ => return Err(Corrupt),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::Encoder;
    use crate::protocol::ENCODING_ZRLE;

    #[test]
    fn what_the_encoder_sends_decodes_to_the_same_picture() {
        let (width, height) = (70u32, 66u32);
        let pixels: Vec<u32> = (0..width * height)
            .map(|index| match (index % width < 64, index / width < 64) {
                (true, true) => {
                    [0x10_2030, 0xa0_b0c0][((index % width + index / width) % 2) as usize]
                }
                (false, true) => index.wrapping_mul(7919) & 0xff_ffff,
                _ => 0x00_ff00,
            })
            .collect();
        let sent = Frame {
            width,
            height,
            pixels,
        };
        let mut encoder = Encoder::default();
        encoder.set_encodings(&[ENCODING_ZRLE]);
        let mut decoder = Decoder::default();
        let mut shown = Frame {
            width,
            height,
            pixels: vec![0; (width * height) as usize],
        };
        for _ in 0..2 {
            let mut bytes = Vec::new();
            encoder.encode(&sent, sent.area(), &mut bytes).unwrap();
            let rectangle = Rectangle {
                area: sent.area(),
                encoding: ENCODING_ZRLE,
                data: bytes[16..].to_vec(),
            };
            decoder.paint(&mut shown, &rectangle).unwrap();
            assert_eq!(shown, sent);
        }
    }
}
