use std::io;

use flate2::{Compress, Compression, FlushCompress, Status};

use crate::rfb::protocol::{
    Area, Compact, ENCODING_DESKTOP_SIZE, ENCODING_RAW, ENCODING_ZRLE, PixelFormat,
    rectangle_header,
};

const TILE: u32 = 64;
const MOST_PALETTE: usize = 16;

/// A picture of the screen, one `0x00RRGGBB` value per pixel, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

impl Frame {
    fn row(&self, y: u32, x: u32, width: u32) -> &[u32] {
        let start = (y * self.width + x) as usize;
        &self.pixels[start..start + width as usize]
    }

    pub fn area(&self) -> Area {
        Area {
            x: 0,
            y: 0,
            width: u16::try_from(self.width).unwrap_or(u16::MAX),
            height: u16::try_from(self.height).unwrap_or(u16::MAX),
        }
    }

    fn clip(&self, area: Area) -> Option<Area> {
        let (x, y) = (u32::from(area.x), u32::from(area.y));
        let right = (x + u32::from(area.width)).min(self.width);
        let bottom = (y + u32::from(area.height)).min(self.height);
        (x < right && y < bottom).then(|| Area {
            x: area.x,
            y: area.y,
            width: (right - x) as u16,
            height: (bottom - y) as u16,
        })
    }
}

pub fn changed_areas(before: Option<&Frame>, now: &Frame, within: Area) -> Vec<Area> {
    let Some(within) = now.clip(within) else {
        return Vec::new();
    };
    let before = before.filter(|before| before.width == now.width && before.height == now.height);
    let Some(before) = before else {
        return vec![within];
    };
    let (left, top) = (u32::from(within.x), u32::from(within.y));
    let right = left + u32::from(within.width);
    let bottom = top + u32::from(within.height);
    let mut areas = Vec::new();
    let mut y = top;
    while y < bottom {
        let height = TILE.min(bottom - y);
        let mut run: Option<(u32, u32)> = None;
        let mut x = left;
        while x <= right {
            let changed = x < right && {
                let width = TILE.min(right - x);
                (y..y + height).any(|row| before.row(row, x, width) != now.row(row, x, width))
            };
            match (changed, run) {
                (true, None) => run = Some((x, x + TILE.min(right - x))),
                (true, Some((start, _))) => run = Some((start, x + TILE.min(right - x))),
                (false, Some((start, end))) => {
                    areas.push(Area {
                        x: start as u16,
                        y: y as u16,
                        width: (end - start) as u16,
                        height: height as u16,
                    });
                    run = None;
                }
                (false, None) => {}
            }
            x += TILE;
        }
        y += TILE;
    }
    areas
}

pub struct Encoder {
    pub format: PixelFormat,
    zrle: bool,
    pub desktop_size: bool,
    zlib: Compress,
}

impl Default for Encoder {
    fn default() -> Self {
        Self {
            format: PixelFormat::NATIVE,
            zrle: false,
            desktop_size: false,
            zlib: Compress::new(Compression::fast(), true),
        }
    }
}

impl Encoder {
    pub fn set_encodings(&mut self, encodings: &[i32]) {
        self.zrle = encodings.contains(&ENCODING_ZRLE);
        self.desktop_size = encodings.contains(&ENCODING_DESKTOP_SIZE);
    }

    pub fn encode(&mut self, frame: &Frame, area: Area, out: &mut Vec<u8>) -> io::Result<()> {
        let Some(area) = frame.clip(area) else {
            return Ok(());
        };
        match self.zrle {
            true => self.zrle(frame, area, out),
            false => {
                rectangle_header(area, ENCODING_RAW, out);
                for y in u32::from(area.y)..u32::from(area.y) + u32::from(area.height) {
                    for &pixel in frame.row(y, u32::from(area.x), u32::from(area.width)) {
                        self.format.put(self.format.value(pixel), out);
                    }
                }
                Ok(())
            }
        }
    }

    fn zrle(&mut self, frame: &Frame, area: Area, out: &mut Vec<u8>) -> io::Result<()> {
        let compact = self.format.compact();
        let mut tiles = Vec::new();
        let mut values = Vec::with_capacity((TILE * TILE) as usize);
        let (left, top) = (u32::from(area.x), u32::from(area.y));
        let (right, bottom) = (left + u32::from(area.width), top + u32::from(area.height));
        for tile_y in (top..bottom).step_by(TILE as usize) {
            let height = TILE.min(bottom - tile_y);
            for tile_x in (left..right).step_by(TILE as usize) {
                let width = TILE.min(right - tile_x);
                values.clear();
                for y in tile_y..tile_y + height {
                    values.extend(
                        frame
                            .row(y, tile_x, width)
                            .iter()
                            .map(|&pixel| self.format.value(pixel)),
                    );
                }
                self.tile(&values, width as usize, compact, &mut tiles);
            }
        }
        let compressed = self.compress(&tiles)?;
        rectangle_header(area, ENCODING_ZRLE, out);
        out.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
        out.extend_from_slice(&compressed);
        Ok(())
    }

    fn tile(&self, values: &[u32], width: usize, compact: Option<Compact>, out: &mut Vec<u8>) {
        let mut palette: Vec<u32> = Vec::with_capacity(MOST_PALETTE);
        for &value in values {
            if !palette.contains(&value) {
                if palette.len() == MOST_PALETTE {
                    palette.clear();
                    break;
                }
                palette.push(value);
            }
        }
        match palette.len() {
            1 => {
                out.push(1);
                self.format.put_compact(palette[0], compact, out);
            }
            0 => {
                out.push(0);
                for &value in values {
                    self.format.put_compact(value, compact, out);
                }
            }
            colours => {
                out.push(colours as u8);
                for &value in &palette {
                    self.format.put_compact(value, compact, out);
                }
                let bits = match colours {
                    2 => 1,
                    3..=4 => 2,
                    _ => 4,
                };
                for row in values.chunks(width) {
                    let mut byte = 0u8;
                    let mut used = 0;
                    for value in row {
                        let index = palette.iter().position(|entry| entry == value).unwrap_or(0);
                        byte |= (index as u8) << (8 - bits - used);
                        used += bits;
                        if used == 8 {
                            out.push(byte);
                            byte = 0;
                            used = 0;
                        }
                    }
                    if used > 0 {
                        out.push(byte);
                    }
                }
            }
        }
    }

    fn compress(&mut self, input: &[u8]) -> io::Result<Vec<u8>> {
        let mut output = Vec::with_capacity(input.len() / 2 + 64);
        let start_in = self.zlib.total_in();
        loop {
            let consumed = (self.zlib.total_in() - start_in) as usize;
            if output.capacity() - output.len() < 64 {
                output.reserve(output.capacity().max(4096));
            }
            let status = self
                .zlib
                .compress_vec(&input[consumed..], &mut output, FlushCompress::Sync)
                .map_err(io::Error::other)?;
            let done = (self.zlib.total_in() - start_in) as usize == input.len();
            if done && output.len() < output.capacity() && status != Status::BufError {
                return Ok(output);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_changed_tiles_are_sent() {
        let before = Frame {
            width: 200,
            height: 100,
            pixels: vec![0; 200 * 100],
        };
        let mut now = before.clone();
        now.pixels[70 * 200 + 130] = 0xff_ffff;
        now.pixels[10 * 200 + 5] = 0x12_3456;
        assert_eq!(
            changed_areas(Some(&before), &now, now.area()),
            vec![
                Area {
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 64
                },
                Area {
                    x: 128,
                    y: 64,
                    width: 64,
                    height: 36
                },
            ]
        );
        assert!(changed_areas(Some(&now), &now, now.area()).is_empty());
    }
}
