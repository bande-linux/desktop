use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use vello_cpu::{PixelMetadata, Pixmap};

use crate::text::Paragraph;
use crate::tokens::Color;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn inset(self, amount: f32) -> Self {
        Self::new(
            self.x + amount,
            self.y + amount,
            (self.width - 2.0 * amount).max(0.0),
            (self.height - 2.0 * amount).max(0.0),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Shape {
    pub rect: Rect,
    pub radius: f32,
}

impl Shape {
    pub const fn rect(rect: Rect) -> Self {
        Self { rect, radius: 0.0 }
    }

    pub const fn rounded(rect: Rect, radius: f32) -> Self {
        Self { rect, radius }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stop {
    pub offset: f32,
    pub color: Color,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    Solid(Color),
    LinearGradient {
        start: Point,
        end: Point,
        stops: Vec<Stop>,
    },
}

impl From<Color> for Paint {
    fn from(color: Color) -> Self {
        Self::Solid(color)
    }
}

#[derive(Clone)]
pub struct Image {
    id: u64,
    pub(crate) pixmap: Arc<Pixmap>,
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image")
            .field("width", &self.width())
            .field("height", &self.height())
            .finish_non_exhaustive()
    }
}

impl Image {
    pub fn from_rgba(width: u32, height: u32, mut rgba: Vec<u8>) -> Option<Self> {
        let (width, height) = (u16::try_from(width).ok()?, u16::try_from(height).ok()?);
        if rgba.len() != usize::from(width) * usize::from(height) * 4 || width == 0 || height == 0 {
            return None;
        }
        for pixel in rgba.as_chunks_mut::<4>().0 {
            let alpha = u16::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Some(Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            pixmap: Arc::new(Pixmap::from_parts(
                rgba,
                width,
                height,
                PixelMetadata::default(),
            )),
        })
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub fn premultiplied_rgba(&self) -> &[u8] {
        self.pixmap.data_as_u8_slice()
    }

    pub fn width(&self) -> u32 {
        u32::from(self.pixmap.width())
    }

    pub fn height(&self) -> u32 {
        u32::from(self.pixmap.height())
    }
}

#[derive(Debug, Clone)]
pub enum Item {
    Fill {
        shape: Shape,
        paint: Paint,
    },
    Stroke {
        shape: Shape,
        width: f32,
        paint: Paint,
    },
    Shadow {
        shape: Shape,
        offset: Point,
        blur_radius: f32,
        color: Color,
    },
    Text {
        origin: Point,
        paragraph: Paragraph,
    },
    Clip {
        shape: Shape,
        items: Vec<Item>,
    },
    Image {
        rect: Rect,
        image: Image,
    },
}

#[derive(Debug, Clone)]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    pub background: Color,
    pub items: Vec<Item>,
}
