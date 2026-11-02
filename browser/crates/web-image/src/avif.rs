const MAX_DEPTH: u8 = 8;

struct Item {
    id: u32,
    kind: [u8; 4],
}

struct Location {
    id: u32,
    method: u8,
    base: u64,
    extents: Vec<(u64, u64)>,
}

#[derive(Default)]
struct Meta {
    primary: Option<u32>,
    items: Vec<Item>,
    locations: Vec<Location>,
    data: Option<(usize, usize)>,
    color: Colour,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Reader { bytes, at: 0 }
    }

    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let slice = self.bytes.get(self.at..self.at.checked_add(count)?)?;
        self.at += count;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1)?.first().copied()
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_be_bytes(self.take(8)?.try_into().ok()?))
    }

    fn sized(&mut self, size: u8) -> Option<u64> {
        match size {
            0 => Some(0),
            4 => self.u32().map(u64::from),
            8 => self.u64(),
            _ => None,
        }
    }

    fn boxes(&mut self) -> impl Iterator<Item = ([u8; 4], usize, &'a [u8])> + '_ {
        std::iter::from_fn(move || {
            let start = self.at;
            let size = self.u32()?;
            let kind: [u8; 4] = self.take(4)?.try_into().ok()?;
            let size = match size {
                1 => usize::try_from(self.u64()?).ok()?,
                0 => self.bytes.len() - start,
                size => size as usize,
            };
            let header = self.at - start;
            let body = size.checked_sub(header)?;
            let offset = self.at;
            let content = self.take(body)?;
            Some((kind, offset, content))
        })
    }
}

fn meta(content: &[u8], meta: &mut Meta, offset: usize, depth: u8) -> Option<()> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut reader = Reader::new(content.get(4..)?);
    for (kind, inner_offset, body) in reader.boxes() {
        let absolute = offset + 4 + inner_offset;
        match &kind {
            b"pitm" => {
                let mut box_reader = Reader::new(body);
                let version = box_reader.u8()?;
                box_reader.take(3)?;
                meta.primary = Some(if version == 0 {
                    u32::from(box_reader.u16()?)
                } else {
                    box_reader.u32()?
                });
            }
            b"iinf" => items(body, meta)?,
            b"iloc" => locations(body, meta)?,
            b"idat" => meta.data = Some((absolute, body.len())),
            b"iprp" => {
                let mut properties = Reader::new(body);
                for (kind, _, container) in properties.boxes() {
                    if &kind != b"ipco" {
                        continue;
                    }
                    let mut container = Reader::new(container);
                    for (kind, _, property) in container.boxes() {
                        let mut property = Reader::new(property);
                        if &kind == b"colr" && property.take(4) == Some(b"nclx") {
                            property.u16()?;
                            property.u16()?;
                            let matrix = property.u16()?;
                            let full = property.u8()? & 0x80 != 0;
                            meta.color = Some((matrix, full));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Some(())
}

fn items(body: &[u8], meta: &mut Meta) -> Option<()> {
    let mut reader = Reader::new(body);
    let version = reader.u8()?;
    reader.take(3)?;
    if version == 0 {
        reader.u16()?;
    } else {
        reader.u32()?;
    }
    for (kind, _, entry) in reader.boxes() {
        if &kind != b"infe" {
            continue;
        }
        let mut entry = Reader::new(entry);
        let version = entry.u8()?;
        entry.take(3)?;
        if version < 2 {
            continue;
        }
        let id = if version == 2 {
            u32::from(entry.u16()?)
        } else {
            entry.u32()?
        };
        entry.u16()?;
        let kind: [u8; 4] = entry.take(4)?.try_into().ok()?;
        meta.items.push(Item { id, kind });
    }
    Some(())
}

fn locations(body: &[u8], meta: &mut Meta) -> Option<()> {
    let mut reader = Reader::new(body);
    let version = reader.u8()?;
    reader.take(3)?;
    let sizes = reader.u8()?;
    let (offset_size, length_size) = (sizes >> 4, sizes & 0xF);
    let more = reader.u8()?;
    let base_size = more >> 4;
    let index_size = if version == 1 || version == 2 {
        more & 0xF
    } else {
        0
    };
    let count = if version < 2 {
        u32::from(reader.u16()?)
    } else {
        reader.u32()?
    };
    for _ in 0..count.min(4096) {
        let id = if version < 2 {
            u32::from(reader.u16()?)
        } else {
            reader.u32()?
        };
        let method = if version == 1 || version == 2 {
            (reader.u16()? & 0xF) as u8
        } else {
            0
        };
        reader.u16()?;
        let base = reader.sized(base_size)?;
        let extents = reader.u16()?;
        let mut list = Vec::new();
        for _ in 0..extents.min(1024) {
            reader.sized(index_size)?;
            let offset = reader.sized(offset_size)?;
            let length = reader.sized(length_size)?;
            list.push((offset, length));
        }
        meta.locations.push(Location {
            id,
            method,
            base,
            extents: list,
        });
    }
    Some(())
}

type Colour = Option<(u16, bool)>;

pub(crate) fn obus(bytes: &[u8]) -> Option<(Vec<u8>, Colour)> {
    let mut reader = Reader::new(bytes);
    let mut found = Meta::default();
    let mut seen_meta = false;
    for (kind, offset, content) in reader.boxes() {
        if &kind == b"meta" {
            meta(content, &mut found, offset, 0)?;
            seen_meta = true;
        }
    }
    if !seen_meta {
        return None;
    }
    let primary = found.primary?;
    let item = found.items.iter().find(|item| item.id == primary)?;
    if &item.kind != b"av01" {
        return None;
    }
    let location = found
        .locations
        .iter()
        .find(|location| location.id == primary)?;
    let mut data = Vec::new();
    for &(offset, length) in &location.extents {
        let start = usize::try_from(location.base.checked_add(offset)?).ok()?;
        let (source, start) = match location.method {
            0 => (bytes, start),
            1 => {
                let (idat, size) = found.data?;
                (bytes.get(idat..idat + size)?, start)
            }
            _ => return None,
        };
        let length = if length == 0 {
            source.len().checked_sub(start)?
        } else {
            usize::try_from(length).ok()?
        };
        data.extend_from_slice(source.get(start..start.checked_add(length)?)?);
    }
    (!data.is_empty()).then_some((data, found.color))
}
