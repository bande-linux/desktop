use std::io::Read;

pub const FONT_LIMIT: usize = 32 << 20;

const KNOWN_TAGS: [&[u8; 4]; 63] = [
    b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post", b"cvt ", b"fpgm",
    b"glyf", b"loca", b"prep", b"CFF ", b"VORG", b"EBDT", b"EBLC", b"gasp", b"hdmx", b"kern",
    b"LTSH", b"PCLT", b"VDMX", b"vhea", b"vmtx", b"BASE", b"GDEF", b"GPOS", b"GSUB", b"EBSC",
    b"JSTF", b"MATH", b"CBDT", b"CBLC", b"COLR", b"CPAL", b"SVG ", b"sbix", b"acnt", b"avar",
    b"bdat", b"bloc", b"bsln", b"cvar", b"fdsc", b"feat", b"fmtx", b"fvar", b"gvar", b"hsty",
    b"just", b"lcar", b"mort", b"morx", b"opbd", b"prop", b"trak", b"Zapf", b"Silf", b"Glat",
    b"Gloc", b"Feat", b"Sill",
];

pub fn decode(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() > FONT_LIMIT {
        return None;
    }
    match bytes.get(..4)? {
        b"wOFF" => woff1(bytes),
        b"wOF2" => woff2(bytes),
        [0, 1, 0, 0] | b"OTTO" | b"true" | b"ttcf" => Some(bytes.to_vec()),
        _ => None,
    }
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
        let end = self.at.checked_add(count)?;
        let slice = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1)?.first().copied()
    }

    fn u16(&mut self) -> Option<u16> {
        let bytes = self.take(2)?;
        Some(u16::from_be_bytes([*bytes.first()?, *bytes.get(1)?]))
    }

    fn i16(&mut self) -> Option<i16> {
        self.u16().map(|value| value as i16)
    }

    fn u32(&mut self) -> Option<u32> {
        let bytes: [u8; 4] = self.take(4)?.try_into().ok()?;
        Some(u32::from_be_bytes(bytes))
    }

    fn tag(&mut self) -> Option<[u8; 4]> {
        self.take(4)?.try_into().ok()
    }

    fn base128(&mut self) -> Option<u32> {
        let mut value: u32 = 0;
        for index in 0..5 {
            let byte = self.u8()?;
            if index == 0 && byte == 0x80 {
                return None;
            }
            if value & 0xfe00_0000 != 0 {
                return None;
            }
            value = (value << 7) | u32::from(byte & 0x7f);
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn u255(&mut self) -> Option<u16> {
        match self.u8()? {
            253 => self.u16(),
            254 => Some(u16::from(self.u8()?) + 506),
            255 => Some(u16::from(self.u8()?) + 253),
            code => Some(u16::from(code)),
        }
    }
}

fn sfnt(flavor: u32, mut tables: Vec<([u8; 4], Vec<u8>)>) -> Option<Vec<u8>> {
    tables.sort_by_key(|(tag, _)| *tag);
    let count = u16::try_from(tables.len()).ok()?;
    let mut power = 1u16;
    let mut log = 0u16;
    while power.checked_mul(2).is_some_and(|next| next <= count) {
        power *= 2;
        log += 1;
    }
    let search_range = power.checked_mul(16)?;
    let total: usize = tables
        .iter()
        .map(|(_, data)| data.len().div_ceil(4) * 4)
        .sum::<usize>()
        + 12
        + 16 * tables.len();
    if total > FONT_LIMIT {
        return None;
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&flavor.to_be_bytes());
    out.extend_from_slice(&count.to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&log.to_be_bytes());
    out.extend_from_slice(&(count.wrapping_mul(16).wrapping_sub(search_range)).to_be_bytes());
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum(data).to_be_bytes());
        out.extend_from_slice(&u32::try_from(offset).ok()?.to_be_bytes());
        out.extend_from_slice(&u32::try_from(data.len()).ok()?.to_be_bytes());
        offset += data.len().div_ceil(4) * 4;
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(4) * 4, 0);
    }
    Some(out)
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        for (slot, byte) in word.iter_mut().zip(chunk) {
            *slot = *byte;
        }
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn woff1(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut header = Reader::new(bytes);
    header.take(4)?;
    let flavor = header.u32()?;
    header.u32()?;
    let count = header.u16()?;
    header.take(2)?;
    let total = header.u32()? as usize;
    if total > FONT_LIMIT {
        return None;
    }
    header.take(24)?;
    let mut tables = Vec::with_capacity(usize::from(count));
    let mut budget = FONT_LIMIT;
    for _ in 0..count {
        let tag = header.tag()?;
        let offset = header.u32()? as usize;
        let compressed = header.u32()? as usize;
        let original = header.u32()? as usize;
        header.u32()?;
        budget = budget.checked_sub(original)?;
        let data = bytes.get(offset..offset.checked_add(compressed)?)?;
        let table = if compressed < original {
            let mut out = Vec::with_capacity(original);
            flate2::read::ZlibDecoder::new(data)
                .take(original as u64)
                .read_to_end(&mut out)
                .ok()?;
            if out.len() != original {
                return None;
            }
            out
        } else if compressed == original {
            data.to_vec()
        } else {
            return None;
        };
        tables.push((tag, table));
    }
    sfnt(flavor, tables)
}

struct Entry {
    tag: [u8; 4],
    length: usize,
    transformed: bool,
}

fn woff2(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut header = Reader::new(bytes);
    header.take(4)?;
    let flavor = header.u32()?;
    if flavor == u32::from_be_bytes(*b"ttcf") {
        return None;
    }
    header.u32()?;
    let count = header.u16()?;
    header.take(2)?;
    header.u32()?;
    let compressed = header.u32()? as usize;
    header.take(24)?;
    let mut entries = Vec::with_capacity(usize::from(count));
    let mut budget = FONT_LIMIT;
    for _ in 0..count {
        let flags = header.u8()?;
        let tag = match flags & 63 {
            63 => header.tag()?,
            index => **KNOWN_TAGS.get(usize::from(index))?,
        };
        let version = flags >> 6;
        let original = header.base128()? as usize;
        let transformed = if &tag == b"glyf" || &tag == b"loca" {
            version == 0
        } else {
            version != 0
        };
        let length = if transformed {
            header.base128()? as usize
        } else {
            original
        };
        budget = budget.checked_sub(original.max(length))?;
        entries.push(Entry {
            tag,
            length,
            transformed,
        });
    }
    let stream = bytes.get(header.at..header.at.checked_add(compressed)?)?;
    let expected: usize = entries.iter().map(|entry| entry.length).sum();
    let mut data = Vec::with_capacity(expected.min(FONT_LIMIT));
    brotli_decompressor::Decompressor::new(stream, 4096)
        .take(expected as u64 + 1)
        .read_to_end(&mut data)
        .ok()?;
    if data.len() != expected {
        return None;
    }
    let mut offset = 0;
    let mut raw: Vec<(&Entry, &[u8])> = Vec::with_capacity(entries.len());
    for entry in &entries {
        raw.push((entry, data.get(offset..offset + entry.length)?));
        offset += entry.length;
    }
    let find = |tag: &[u8; 4]| raw.iter().find(|(entry, _)| &entry.tag == tag);
    let mut tables = Vec::with_capacity(raw.len());
    let mut glyf = None;
    if let Some((_, table)) = find(b"glyf").filter(|(entry, _)| entry.transformed) {
        let Glyf {
            glyphs,
            loca,
            x_mins,
            short,
        } = reconstruct_glyf(table)?;
        if let Some((_, head)) = find(b"head") {
            let mut head = head.to_vec();
            if let Some(format) = head.get_mut(50..52) {
                format.copy_from_slice(&[0, u8::from(!short)]);
            }
            tables.push((*b"head", head));
        }
        tables.push((*b"glyf", glyphs));
        tables.push((*b"loca", loca));
        glyf = Some(x_mins);
    }
    for (entry, table) in &raw {
        match &entry.tag {
            b"glyf" | b"loca" | b"head" if glyf.is_some() => {}
            b"glyf" | b"loca" if entry.transformed => return None,
            b"hmtx" if entry.transformed => {
                let x_mins = glyf.as_ref()?;
                let hhea = find(b"hhea")?.1;
                let metrics = u16::from_be_bytes([*hhea.get(34)?, *hhea.get(35)?]);
                tables.push((*b"hmtx", reconstruct_hmtx(table, metrics, x_mins)?));
            }
            _ if entry.transformed => return None,
            tag => tables.push((*tag, table.to_vec())),
        }
    }
    sfnt(flavor, tables)
}

struct Glyf {
    glyphs: Vec<u8>,
    loca: Vec<u8>,
    x_mins: Vec<i16>,
    short: bool,
}

fn reconstruct_glyf(table: &[u8]) -> Option<Glyf> {
    let mut header = Reader::new(table);
    header.u16()?;
    let options = header.u16()?;
    let glyph_count = header.u16()?;
    let index_format = header.u16()?;
    let mut sizes = [0usize; 7];
    for size in &mut sizes {
        *size = header.u32()? as usize;
    }
    let mut streams = Vec::with_capacity(7);
    for size in sizes {
        streams.push(Reader::new(header.take(size)?));
    }
    let mut streams = streams.into_iter();
    let (
        Some(mut contours),
        Some(mut points),
        Some(mut flags),
        Some(mut glyph_stream),
        Some(mut composite),
        Some(bbox_stream),
        Some(mut instructions),
    ) = (
        streams.next(),
        streams.next(),
        streams.next(),
        streams.next(),
        streams.next(),
        streams.next(),
        streams.next(),
    )
    else {
        return None;
    };
    let mut bbox = bbox_stream;
    let bitmap = bbox.take(usize::from(glyph_count).div_ceil(32) * 4)?;
    let overlap = if options & 1 != 0 {
        Some(header.take(usize::from(glyph_count).div_ceil(8))?)
    } else {
        None
    };
    let has_bbox = |glyph: usize| {
        bitmap
            .get(glyph / 8)
            .is_some_and(|byte| byte & (0x80 >> (glyph % 8)) != 0)
    };
    let mut out = Vec::new();
    let mut offsets = Vec::with_capacity(usize::from(glyph_count) + 1);
    let mut x_mins = Vec::with_capacity(usize::from(glyph_count));
    for glyph in 0..usize::from(glyph_count) {
        offsets.push(out.len());
        let count = contours.i16()?;
        let explicit = if has_bbox(glyph) {
            Some([bbox.i16()?, bbox.i16()?, bbox.i16()?, bbox.i16()?])
        } else {
            None
        };
        if count == 0 {
            if explicit.is_some() {
                return None;
            }
            x_mins.push(0);
            continue;
        }
        if count < 0 {
            let bounds = explicit?;
            out.extend_from_slice(&count.to_be_bytes());
            for value in bounds {
                out.extend_from_slice(&value.to_be_bytes());
            }
            let mut has_instructions = false;
            loop {
                let component = composite.u16()?;
                let mut length = 4;
                length += if component & 1 != 0 { 4 } else { 2 };
                length += if component & 0x8 != 0 {
                    2
                } else if component & 0x40 != 0 {
                    4
                } else if component & 0x80 != 0 {
                    8
                } else {
                    0
                };
                out.extend_from_slice(&component.to_be_bytes());
                out.extend_from_slice(composite.take(length - 2)?);
                has_instructions |= component & 0x100 != 0;
                if component & 0x20 == 0 {
                    break;
                }
            }
            if has_instructions {
                let length = glyph_stream.u255()?;
                out.extend_from_slice(&length.to_be_bytes());
                out.extend_from_slice(instructions.take(usize::from(length))?);
            }
            x_mins.push(bounds[0]);
        } else {
            let mut ends = Vec::with_capacity(count as usize);
            let mut total: usize = 0;
            for _ in 0..count {
                total = total.checked_add(usize::from(points.u255()?))?;
                ends.push(u16::try_from(total.checked_sub(1)?).ok()?);
            }
            let mut coordinates = Vec::with_capacity(total);
            let (mut x, mut y) = (0i32, 0i32);
            for _ in 0..total {
                let flag = flags.u8()?;
                let on_curve = flag & 0x80 == 0;
                let code = i32::from(flag & 0x7f);
                let sign = |flag: i32, value: i32| if flag & 1 != 0 { value } else { -value };
                let byte = |reader: &mut Reader<'_>| reader.u8().map(i32::from);
                let (dx, dy) = if code < 10 {
                    (0, sign(code, ((code & 14) << 7) + byte(&mut glyph_stream)?))
                } else if code < 20 {
                    (
                        sign(code, (((code - 10) & 14) << 7) + byte(&mut glyph_stream)?),
                        0,
                    )
                } else if code < 84 {
                    let b0 = code - 20;
                    let b1 = byte(&mut glyph_stream)?;
                    (
                        sign(code, 1 + (b0 & 0x30) + (b1 >> 4)),
                        sign(code >> 1, 1 + ((b0 & 0x0c) << 2) + (b1 & 0x0f)),
                    )
                } else if code < 120 {
                    let b0 = code - 84;
                    let first = byte(&mut glyph_stream)?;
                    let second = byte(&mut glyph_stream)?;
                    (
                        sign(code, 1 + ((b0 / 12) << 8) + first),
                        sign(code >> 1, 1 + (((b0 % 12) >> 2) << 8) + second),
                    )
                } else if code < 124 {
                    let first = byte(&mut glyph_stream)?;
                    let second = byte(&mut glyph_stream)?;
                    let third = byte(&mut glyph_stream)?;
                    (
                        sign(code, (first << 4) + (second >> 4)),
                        sign(code >> 1, ((second & 0x0f) << 8) + third),
                    )
                } else {
                    let first = byte(&mut glyph_stream)?;
                    let second = byte(&mut glyph_stream)?;
                    let third = byte(&mut glyph_stream)?;
                    let fourth = byte(&mut glyph_stream)?;
                    (
                        sign(code, (first << 8) + second),
                        sign(code >> 1, (third << 8) + fourth),
                    )
                };
                x = x.checked_add(dx)?;
                y = y.checked_add(dy)?;
                coordinates.push((i16::try_from(x).ok()?, i16::try_from(y).ok()?, on_curve));
            }
            let length = glyph_stream.u255()?;
            let code = instructions.take(usize::from(length))?;
            let bounds = match explicit {
                Some(bounds) => bounds,
                None => coordinates.iter().fold(
                    [i16::MAX, i16::MAX, i16::MIN, i16::MIN],
                    |[x0, y0, x1, y1], &(x, y, _)| [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
                ),
            };
            out.extend_from_slice(&count.to_be_bytes());
            for value in bounds {
                out.extend_from_slice(&value.to_be_bytes());
            }
            for end in ends {
                out.extend_from_slice(&end.to_be_bytes());
            }
            out.extend_from_slice(&length.to_be_bytes());
            out.extend_from_slice(code);
            let overlaps = overlap.is_some_and(|bits| {
                bits.get(glyph / 8)
                    .is_some_and(|byte| byte & (0x80 >> (glyph % 8)) != 0)
            });
            write_points(&mut out, &coordinates, overlaps);
            x_mins.push(bounds[0]);
        }
        out.resize(out.len().div_ceil(4) * 4, 0);
        if out.len() > FONT_LIMIT {
            return None;
        }
    }
    offsets.push(out.len());
    let short = index_format == 0 && out.len() / 2 <= usize::from(u16::MAX);
    let mut loca = Vec::with_capacity(offsets.len() * 4);
    for offset in offsets {
        if short {
            loca.extend_from_slice(&u16::try_from(offset / 2).ok()?.to_be_bytes());
        } else {
            loca.extend_from_slice(&u32::try_from(offset).ok()?.to_be_bytes());
        }
    }
    Some(Glyf {
        glyphs: out,
        loca,
        x_mins,
        short,
    })
}

fn write_points(out: &mut Vec<u8>, coordinates: &[(i16, i16, bool)], overlaps: bool) {
    let mut flags = Vec::with_capacity(coordinates.len());
    let mut xs = Vec::with_capacity(coordinates.len() * 2);
    let mut ys = Vec::with_capacity(coordinates.len() * 2);
    let (mut previous_x, mut previous_y) = (0i16, 0i16);
    for (index, &(x, y, on_curve)) in coordinates.iter().enumerate() {
        let mut flag = u8::from(on_curve);
        if index == 0 && overlaps {
            flag |= 0x40;
        }
        let dx = i32::from(x) - i32::from(previous_x);
        let dy = i32::from(y) - i32::from(previous_y);
        for (delta, short, same, bytes) in [(dx, 0x02, 0x10, &mut xs), (dy, 0x04, 0x20, &mut ys)] {
            if delta == 0 {
                flag |= same;
            } else if delta.abs() < 256 {
                flag |= short;
                if delta > 0 {
                    flag |= same;
                }
                bytes.push(delta.unsigned_abs() as u8);
            } else {
                bytes.extend_from_slice(&(delta as i16).to_be_bytes());
            }
        }
        flags.push(flag);
        previous_x = x;
        previous_y = y;
    }
    let mut index = 0;
    while let Some(&flag) = flags.get(index) {
        let mut run = 1;
        while run < 256 && flags.get(index + run) == Some(&flag) {
            run += 1;
        }
        if run > 1 {
            out.push(flag | 0x08);
            out.push((run - 1) as u8);
        } else {
            out.push(flag);
        }
        index += run;
    }
    out.extend_from_slice(&xs);
    out.extend_from_slice(&ys);
}

fn reconstruct_hmtx(table: &[u8], metrics: u16, x_mins: &[i16]) -> Option<Vec<u8>> {
    let mut reader = Reader::new(table);
    let flags = reader.u8()?;
    let glyphs = x_mins.len();
    let metrics = usize::from(metrics);
    if metrics == 0 || metrics > glyphs {
        return None;
    }
    let mut advances = Vec::with_capacity(metrics);
    for _ in 0..metrics {
        advances.push(reader.u16()?);
    }
    let mut out = Vec::with_capacity(metrics * 4 + (glyphs - metrics) * 2);
    let mut bearings = Vec::with_capacity(glyphs);
    for glyph in 0..metrics {
        bearings.push(if flags & 1 == 0 {
            reader.i16()?
        } else {
            *x_mins.get(glyph)?
        });
    }
    for glyph in metrics..glyphs {
        bearings.push(if flags & 2 == 0 {
            reader.i16()?
        } else {
            *x_mins.get(glyph)?
        });
    }
    for (glyph, bearing) in bearings.iter().enumerate() {
        if let Some(advance) = advances.get(glyph) {
            out.extend_from_slice(&advance.to_be_bytes());
        }
        out.extend_from_slice(&bearing.to_be_bytes());
    }
    Some(out)
}
