use std::path::Path;
use std::sync::Arc;

use bande_content::Failure;

use crate::canvas::Orientation;
use crate::copy::encode_png;

const SOI: [u8; 2] = [0xFF, 0xD8];
const APP0: u8 = 0xE0;
const APP1: u8 = 0xE1;
const SOS: u8 = 0xDA;
const EXIF: &[u8; 6] = b"Exif\0\0";
const ORIENTATION: u16 = 0x0112;
const SHORT: u16 = 3;
const ENTRY: usize = 12;
const MAX_SEGMENT: usize = 0xFFFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Unreadable,
    Unwritable,
    Unusual,
}

pub fn exif_value(orientation: Orientation) -> u16 {
    match (orientation.turns % 4, orientation.flipped) {
        (0, false) => 1,
        (0, true) => 2,
        (2, false) => 3,
        (2, true) => 4,
        (1, true) => 5,
        (1, false) => 6,
        (3, true) => 7,
        _ => 8,
    }
}

fn from_exif(value: u16) -> Orientation {
    let (turns, flipped) = match value {
        2 => (0, true),
        3 => (2, false),
        4 => (2, true),
        5 => (1, true),
        6 => (1, false),
        7 => (3, true),
        8 => (3, false),
        _ => (0, false),
    };
    Orientation { turns, flipped }
}

pub fn after(first: Orientation, then: Orientation) -> Orientation {
    let turned = match first.flipped {
        true => (4 - then.turns % 4) % 4,
        false => then.turns % 4,
    };
    Orientation {
        turns: (turned + first.turns) % 4,
        flipped: first.flipped != then.flipped,
    }
}

#[derive(Clone, Copy)]
struct Order(bool);

impl Order {
    fn u16(self, bytes: &[u8], at: usize) -> Option<u16> {
        let pair: [u8; 2] = bytes.get(at..at + 2)?.try_into().ok()?;
        Some(match self.0 {
            true => u16::from_be_bytes(pair),
            false => u16::from_le_bytes(pair),
        })
    }

    fn u32(self, bytes: &[u8], at: usize) -> Option<u32> {
        let four: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
        Some(match self.0 {
            true => u32::from_be_bytes(four),
            false => u32::from_le_bytes(four),
        })
    }

    fn put_u16(self, value: u16) -> [u8; 2] {
        match self.0 {
            true => value.to_be_bytes(),
            false => value.to_le_bytes(),
        }
    }

    fn put_u32(self, value: u32) -> [u8; 4] {
        match self.0 {
            true => value.to_be_bytes(),
            false => value.to_le_bytes(),
        }
    }
}

struct Segment {
    start: usize,
    marker: u8,
    end: usize,
}

fn segments(jpeg: &[u8]) -> Option<Vec<Segment>> {
    if jpeg.get(..2)? != SOI {
        return None;
    }
    let mut found = Vec::new();
    let mut at = 2;
    loop {
        if *jpeg.get(at)? != 0xFF {
            return None;
        }
        let marker = *jpeg.get(at + 1)?;
        if marker == SOS {
            return Some(found);
        }
        let length = usize::from(u16::from_be_bytes([*jpeg.get(at + 2)?, *jpeg.get(at + 3)?]));
        let end = at + 2 + length;
        if length < 2 || end > jpeg.len() {
            return None;
        }
        found.push(Segment {
            start: at,
            marker,
            end,
        });
        at = end;
    }
}

fn entry(order: Order, tag: u16, value: u16) -> [u8; ENTRY] {
    let mut entry = [0; ENTRY];
    entry[0..2].copy_from_slice(&order.put_u16(tag));
    entry[2..4].copy_from_slice(&order.put_u16(SHORT));
    entry[4..8].copy_from_slice(&order.put_u32(1));
    entry[8..10].copy_from_slice(&order.put_u16(value));
    entry
}

fn oriented_tiff(tiff: &[u8], turn: Orientation) -> Option<Vec<u8>> {
    let order = match tiff.get(..2)? {
        b"MM" => Order(true),
        b"II" => Order(false),
        _ => return None,
    };
    if order.u16(tiff, 2)? != 42 {
        return None;
    }
    let ifd = usize::try_from(order.u32(tiff, 4)?).ok()?;
    let count = usize::from(order.u16(tiff, ifd)?);
    let entries = ifd + 2;
    let next = entries + count * ENTRY;
    tiff.get(next..next + 4)?;
    let tags: Vec<u16> = (0..count)
        .map(|index| order.u16(tiff, entries + index * ENTRY))
        .collect::<Option<_>>()?;
    let mut changed = tiff.to_vec();
    if let Some(index) = tags.iter().position(|tag| *tag == ORIENTATION) {
        let at = entries + index * ENTRY;
        if order.u16(tiff, at + 2)? != SHORT || order.u32(tiff, at + 4)? != 1 {
            return None;
        }
        let value = after(from_exif(order.u16(tiff, at + 8)?), turn);
        changed[at + 8..at + 10].copy_from_slice(&order.put_u16(exif_value(value)));
        return Some(changed);
    }
    let place = tags.partition_point(|tag| *tag < ORIENTATION);
    changed.resize(changed.len().next_multiple_of(2), 0);
    let moved = u32::try_from(changed.len()).ok()?;
    changed.extend_from_slice(&order.put_u16(u16::try_from(count + 1).ok()?));
    changed.extend_from_slice(tiff.get(entries..entries + place * ENTRY)?);
    changed.extend_from_slice(&entry(order, ORIENTATION, exif_value(turn)));
    changed.extend_from_slice(tiff.get(entries + place * ENTRY..next + 4)?);
    changed[4..8].copy_from_slice(&order.put_u32(moved));
    Some(changed)
}

fn new_tiff(turn: Orientation) -> Vec<u8> {
    let order = Order(true);
    let mut tiff = b"MM\0\x2A".to_vec();
    tiff.extend_from_slice(&order.put_u32(8));
    tiff.extend_from_slice(&order.put_u16(1));
    tiff.extend_from_slice(&entry(order, ORIENTATION, exif_value(turn)));
    tiff.extend_from_slice(&order.put_u32(0));
    tiff
}

pub fn orient_jpeg(jpeg: &[u8], turn: Orientation) -> Option<Vec<u8>> {
    let segments = segments(jpeg)?;
    let exif = segments.iter().find(|segment| {
        segment.marker == APP1 && jpeg.get(segment.start + 4..segment.start + 10) == Some(EXIF)
    });
    let (start, end, tiff) = match exif {
        Some(segment) => (
            segment.start,
            segment.end,
            oriented_tiff(jpeg.get(segment.start + 10..segment.end)?, turn)?,
        ),
        None => {
            let at = segments
                .iter()
                .take_while(|segment| segment.marker == APP0)
                .last()
                .map_or(2, |segment| segment.end);
            (at, at, new_tiff(turn))
        }
    };
    let length = 2 + EXIF.len() + tiff.len();
    if length > MAX_SEGMENT {
        return None;
    }
    let mut written = Vec::with_capacity(jpeg.len() + length + 2);
    written.extend_from_slice(jpeg.get(..start)?);
    written.extend_from_slice(&[0xFF, APP1]);
    written.extend_from_slice(&u16::try_from(length).ok()?.to_be_bytes());
    written.extend_from_slice(EXIF);
    written.extend_from_slice(&tiff);
    written.extend_from_slice(jpeg.get(end..)?);
    Some(written)
}

pub fn save_jpeg_orientation(path: &Path, turn: Orientation) -> Result<(), Refusal> {
    let bytes = bande_content::read_file(path).map_err(|failure| match failure {
        Failure::FileTooLarge => Refusal::Unusual,
        _ => Refusal::Unreadable,
    })?;
    let written = orient_jpeg(&bytes, turn).ok_or(Refusal::Unusual)?;
    bande_xdg::replace_file(path, written).map_err(|error| {
        tracing::warn!("cannot save the rotation of {}: {error}", path.display());
        Refusal::Unwritable
    })
}

pub fn save_png_copy(
    path: &Path,
    source: (u32, u32, Arc<Vec<u8>>),
    turn: Orientation,
) -> Result<(), Refusal> {
    let png = encode_png(source, turn).ok_or(Refusal::Unwritable)?;
    bande_xdg::replace_file(path, png).map_err(|error| {
        tracing::warn!("cannot save {}: {error}", path.display());
        Refusal::Unwritable
    })
}
