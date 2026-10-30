use std::fmt;
use std::fs;
use std::path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use resvg::{tiny_skia, usvg};
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

    pub fn bands(&self, count: usize) -> Vec<Rect> {
        let Self { rect, radius } = *self;
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return Vec::new();
        }
        let radius = radius.min(rect.width / 2.0).min(rect.height / 2.0).max(0.0);
        let count = if radius > 0.0 { count.max(1) } else { 0 };
        let band = radius / count.max(1) as f32;
        let mut rects = vec![Rect::new(
            rect.x,
            rect.y + radius,
            rect.width,
            rect.height - 2.0 * radius,
        )];
        for index in 0..count {
            let from_edge = index as f32 * band;
            let rise = radius - from_edge;
            let inset = (radius - (radius * radius - rise * rise).max(0.0).sqrt()).ceil();
            let width = rect.width - 2.0 * inset;
            rects.push(Rect::new(rect.x + inset, rect.y + from_edge, width, band));
            rects.push(Rect::new(
                rect.x + inset,
                rect.y + rect.height - from_edge - band,
                width,
                band,
            ));
        }
        rects
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PathElement {
    Move(Point),
    Line(Point),
    Cubic(Point, Point, Point),
    Close,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Path {
    pub(crate) elements: Vec<PathElement>,
}

impl Path {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn through(points: &[Point]) -> Self {
        let mut path = Self::new();
        for (index, point) in points.iter().enumerate() {
            path = match index {
                0 => path.move_to(*point),
                _ => path.line_to(*point),
            };
        }
        path
    }

    pub fn move_to(mut self, point: Point) -> Self {
        self.elements.push(PathElement::Move(point));
        self
    }

    pub fn line_to(mut self, point: Point) -> Self {
        self.elements.push(PathElement::Line(point));
        self
    }

    pub fn cubic_to(mut self, first: Point, second: Point, end: Point) -> Self {
        self.elements.push(PathElement::Cubic(first, second, end));
        self
    }

    pub fn close(mut self) -> Self {
        self.elements.push(PathElement::Close);
        self
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
    alive: Arc<()>,
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
    pub fn load(path: &path::Path, size: u32) -> Option<Self> {
        let bytes = fs::read(path).ok()?;
        if path.extension().is_some_and(|extension| extension == "svg") {
            return Self::from_svg(&bytes, size, size);
        }
        let decoded = image::load_from_memory(&bytes).ok()?.to_rgba8();
        let (width, height) = decoded.dimensions();
        Self::from_rgba(width, height, decoded.into_raw())
    }

    pub fn from_svg(bytes: &[u8], width: u32, height: u32) -> Option<Self> {
        let tree = usvg::Tree::from_data(bytes, &usvg::Options::default()).ok()?;
        let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
        let view = tree.size();
        let scale = (width as f32 / view.width()).min(height as f32 / view.height());
        let offset_x = (width as f32 - view.width() * scale) / 2.0;
        let offset_y = (height as f32 - view.height() * scale) / 2.0;
        resvg::render(
            &tree,
            tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, offset_x, offset_y),
            &mut pixmap.as_mut(),
        );
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for pixel in pixmap.pixels() {
            let color = pixel.demultiply();
            rgba.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
        }
        Self::from_rgba(width, height, rgba)
    }

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
            alive: Arc::new(()),
        })
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn liveness(&self) -> Weak<()> {
        Arc::downgrade(&self.alive)
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
    Translate {
        offset: Point,
        items: Vec<Item>,
    },
    Opacity {
        alpha: f32,
        items: Vec<Item>,
    },
    Line {
        from: Point,
        to: Point,
        width: f32,
        paint: Paint,
    },
    FillPath {
        path: Path,
        paint: Paint,
    },
    StrokePath {
        path: Path,
        width: f32,
        paint: Paint,
    },
    Scale {
        origin: Point,
        factor: f32,
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
