use url::Url;
use web_paint::{Color, DisplayList, Item, Rect};
use web_text::Glyph;

use crate::wire::{Malformed, Reader, Wire, Writer};

pub const MAX_LIST: usize = 256 << 20;

pub struct Painted {
    pub width: f32,
    pub height: f32,
    pub background: Color,
    pub items: Vec<Item>,
    pub glyphs: Vec<Glyph>,
    pub fonts: Vec<u32>,
    pub images: Vec<u32>,
    pub links: Vec<(Rect, u32)>,
    pub targets: Vec<Url>,
}

pub type LinkTable = (Vec<(Rect, u32)>, Vec<Url>);

pub struct Splice<T> {
    pub start: u32,
    pub removed: u32,
    pub inserted: Vec<T>,
}

impl<T: Clone> Splice<T> {
    fn apply(&self, base: &[T]) -> Result<Vec<T>, Malformed> {
        let start = self.start as usize;
        let end = start.checked_add(self.removed as usize).ok_or(Malformed)?;
        let (Some(head), Some(tail)) = (base.get(..start), base.get(end..)) else {
            return Err(Malformed);
        };
        let mut merged = Vec::with_capacity(head.len() + self.inserted.len() + tail.len());
        merged.extend_from_slice(head);
        merged.extend_from_slice(&self.inserted);
        merged.extend_from_slice(tail);
        Ok(merged)
    }

    pub fn removed_range(&self) -> std::ops::Range<usize> {
        self.start as usize..self.start as usize + self.removed as usize
    }

    pub fn inserted_range(&self) -> std::ops::Range<usize> {
        self.start as usize..self.start as usize + self.inserted.len()
    }
}

impl<T: Wire> Wire for Splice<T> {
    fn write(&self, writer: &mut Writer) {
        writer.put(&self.start);
        writer.put(&self.removed);
        writer.put(&self.inserted);
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, Malformed> {
        Ok(Splice {
            start: reader.get()?,
            removed: reader.get()?,
            inserted: reader.get()?,
        })
    }
}

fn write_splice<T: Wire + PartialEq>(writer: &mut Writer, base: &[T], new: &[T]) {
    let prefix = base
        .iter()
        .zip(new)
        .take_while(|(old, new)| old == new)
        .count();
    let suffix = base[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(old, new)| old == new)
        .count();
    let inserted = new.get(prefix..new.len() - suffix).unwrap_or_default();
    writer.put(&(prefix as u32));
    writer.put(&((base.len() - prefix - suffix) as u32));
    writer.put(&(inserted.len() as u32));
    for value in inserted {
        writer.put(value);
    }
}

pub struct Patch {
    pub width: f32,
    pub height: f32,
    pub background: Color,
    pub items: Splice<Item>,
    pub glyphs: Splice<Glyph>,
    pub fonts: Vec<u32>,
    pub images: Vec<u32>,
    pub links: Option<LinkTable>,
}

pub struct Sent<'a> {
    pub list: &'a DisplayList,
    pub links: &'a [(Rect, u32)],
    pub targets: &'a [Url],
}

impl Patch {
    pub fn encode(
        base: Option<&Sent<'_>>,
        sent: &Sent<'_>,
        fonts: &[u32],
        images: &[u32],
    ) -> Vec<u8> {
        let list = sent.list;
        let mut writer = Writer::default();
        writer.put(&list.width);
        writer.put(&list.height);
        writer.put(&list.background);
        write_splice(
            &mut writer,
            base.map_or(&[], |base| &base.list.items),
            &list.items,
        );
        write_splice(
            &mut writer,
            base.map_or(&[], |base| &base.list.glyphs),
            &list.glyphs,
        );
        writer.put(&fonts.to_vec());
        writer.put(&images.to_vec());
        let unchanged =
            base.is_some_and(|base| base.links == sent.links && base.targets == sent.targets);
        match unchanged {
            true => writer.put(&0u8),
            false => {
                writer.put(&1u8);
                writer.put(&sent.links.to_vec());
                writer.put(&sent.targets.to_vec());
            }
        }
        writer.into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Result<Patch, Malformed> {
        let mut reader = Reader::new(bytes);
        let patch = Patch {
            width: reader.get()?,
            height: reader.get()?,
            background: reader.get()?,
            items: reader.get()?,
            glyphs: reader.get()?,
            fonts: reader.get()?,
            images: reader.get()?,
            links: reader.get()?,
        };
        reader.finish()?;
        Ok(patch)
    }

    pub fn apply(
        self,
        items: &[Item],
        glyphs: &[Glyph],
        links: (&[(Rect, u32)], &[Url]),
    ) -> Result<Painted, Malformed> {
        let (links, targets) = match self.links {
            Some(links) => links,
            None => (links.0.to_vec(), links.1.to_vec()),
        };
        let painted = Painted {
            width: self.width,
            height: self.height,
            background: self.background,
            items: self.items.apply(items)?,
            glyphs: self.glyphs.apply(glyphs)?,
            fonts: self.fonts,
            images: self.images,
            links,
            targets,
        };
        painted.validate().then_some(painted).ok_or(Malformed)
    }
}

impl Painted {
    fn validate(&self) -> bool {
        let fonts = self.fonts.len();
        let images = self.images.len();
        let glyphs = self.glyphs.len();
        let mut depth = 0usize;
        let items = self.items.iter().all(|item| match item {
            Item::Glyphs {
                font,
                glyphs: range,
                ..
            } => {
                (*font as usize) < fonts && range.start <= range.end && range.end as usize <= glyphs
            }
            Item::Image { image, .. } | Item::Tiles { image, .. } => (*image as usize) < images,
            Item::PushClip { .. } | Item::PushOpacity { .. } => {
                depth += 1;
                true
            }
            Item::Pop => match depth.checked_sub(1) {
                Some(open) => {
                    depth = open;
                    true
                }
                None => false,
            },
            Item::Rect { .. } => true,
        });
        let links = self
            .links
            .iter()
            .all(|(_, target)| (*target as usize) < self.targets.len());
        items && links && depth == 0
    }
}

impl Wire for Rect {
    fn write(&self, writer: &mut Writer) {
        writer.put(&self.x);
        writer.put(&self.y);
        writer.put(&self.width);
        writer.put(&self.height);
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, Malformed> {
        Ok(Rect {
            x: reader.get()?,
            y: reader.get()?,
            width: reader.get()?,
            height: reader.get()?,
        })
    }
}

impl Wire for Color {
    fn write(&self, writer: &mut Writer) {
        writer.put(&u32::from_le_bytes([self.r, self.g, self.b, self.a]));
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, Malformed> {
        let [r, g, b, a] = reader.get::<u32>()?.to_le_bytes();
        Ok(Color { r, g, b, a })
    }
}

impl Wire for Glyph {
    fn write(&self, writer: &mut Writer) {
        writer.put(&self.id);
        writer.put(&self.x);
        writer.put(&self.y());
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, Malformed> {
        Ok(Glyph::new(reader.get()?, reader.get()?, reader.get()?))
    }
}

impl Wire for Item {
    fn write(&self, writer: &mut Writer) {
        match self {
            Item::Rect { rect, color } => {
                writer.put(&0u8);
                writer.put(rect);
                writer.put(color);
            }
            Item::Glyphs {
                rect,
                baseline,
                font,
                size,
                color,
                glyphs,
                ..
            } => {
                writer.put(&1u8);
                writer.put(rect);
                writer.put(baseline);
                writer.put(font);
                writer.put(size);
                writer.put(color);
                writer.put(&glyphs.start);
                writer.put(&glyphs.end);
            }
            Item::Image { rect, clip, image } => {
                writer.put(&2u8);
                writer.put(rect);
                writer.put(clip);
                writer.put(image);
            }
            Item::Tiles { tile, clip, image } => {
                writer.put(&3u8);
                writer.put(tile);
                writer.put(clip);
                writer.put(image);
            }
            Item::PushClip { rect } => {
                writer.put(&4u8);
                writer.put(rect);
            }
            Item::PushOpacity { alpha } => {
                writer.put(&5u8);
                writer.put(alpha);
            }
            Item::Pop => writer.put(&6u8),
        }
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, Malformed> {
        Ok(match reader.get::<u8>()? {
            0 => Item::Rect {
                rect: reader.get()?,
                color: reader.get()?,
            },
            1 => Item::Glyphs {
                rect: reader.get()?,
                baseline: reader.get()?,
                node: None,
                font: reader.get()?,
                size: reader.get()?,
                color: reader.get()?,
                glyphs: reader.get()?..reader.get()?,
            },
            2 => Item::Image {
                rect: reader.get()?,
                clip: reader.get()?,
                image: reader.get()?,
            },
            3 => Item::Tiles {
                tile: reader.get()?,
                clip: reader.get()?,
                image: reader.get()?,
            },
            4 => Item::PushClip {
                rect: reader.get()?,
            },
            5 => Item::PushOpacity {
                alpha: reader.get()?,
            },
            6 => Item::Pop,
            _ => return Err(Malformed),
        })
    }
}
