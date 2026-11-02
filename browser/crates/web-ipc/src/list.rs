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

impl Painted {
    pub fn encode(
        list: &DisplayList,
        fonts: Vec<u32>,
        images: Vec<u32>,
        links: Vec<(Rect, u32)>,
        targets: Vec<Url>,
    ) -> Vec<u8> {
        let mut writer = Writer::default();
        writer.put(&list.width);
        writer.put(&list.height);
        writer.put(&list.background);
        (list.items.len() as u32).write(&mut writer);
        for item in &list.items {
            writer.put(item);
        }
        (list.glyphs.len() as u32).write(&mut writer);
        for glyph in &list.glyphs {
            writer.put(glyph);
        }
        writer.put(&fonts);
        writer.put(&images);
        writer.put(&links);
        writer.put(&targets);
        writer.into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Result<Painted, Malformed> {
        let mut reader = Reader::new(bytes);
        let painted = Painted {
            width: reader.get()?,
            height: reader.get()?,
            background: reader.get()?,
            items: reader.get()?,
            glyphs: reader.get()?,
            fonts: reader.get()?,
            images: reader.get()?,
            links: reader.get()?,
            targets: reader.get()?,
        };
        reader.finish()?;
        painted.validate().then_some(painted).ok_or(Malformed)
    }

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
