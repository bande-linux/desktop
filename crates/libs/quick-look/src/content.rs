use std::time::Instant;

use bande_content::{Block, Code, Content, Contents, Frame, Level, Pixels};
use bande_toolkit::Color;

use crate::loader::Loaded;
use crate::worker::Sample;
use bande_toolkit::{Image, Paragraph};

use crate::pdf::PdfView;

pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub levels: Vec<Level>,
    pub frames: Vec<Frame>,
    pub frame: usize,
    pub next: Option<Instant>,
    pub shown: Option<(usize, Image)>,
}

impl Picture {
    pub fn image(&mut self, wanted_width: f32) -> Option<Image> {
        let index = match self.frames.is_empty() {
            true => self
                .levels
                .iter()
                .rposition(|level| level.width as f32 >= wanted_width)
                .unwrap_or(0),
            false => self.frame,
        };
        if let Some((shown, image)) = &self.shown
            && *shown == index
        {
            return Some(image.clone());
        }
        let image = match self.frames.get(index) {
            Some(frame) => Image::from_rgba(self.width, self.height, frame.rgba.to_vec()),
            None => {
                let level = self.levels.get(index)?;
                Image::from_rgba(level.width, level.height, level.rgba.to_vec())
            }
        }?;
        self.shown = Some((index, image.clone()));
        Some(image)
    }
}

pub struct CodeView {
    pub code: Code,
    pub starts: Vec<usize>,
    pub widest: usize,
    pub laid: Option<((usize, usize), Paragraph)>,
}

pub type Laid<T> = Option<(f32, Vec<T>, f32)>;

pub struct DocumentView {
    pub blocks: Vec<Block>,
    pub laid: Laid<(f32, Block, Paragraph)>,
}

pub struct FontView {
    pub sample: Image,
    pub tinted: Option<(Color, Image)>,
}

impl FontView {
    pub fn image(&mut self, color: Color) -> Option<Image> {
        if let Some((tint, image)) = &self.tinted
            && *tint == color
        {
            return Some(image.clone());
        }
        let image = self.sample.tinted(color)?;
        self.tinted = Some((color, image.clone()));
        Some(image)
    }
}

pub enum Shown {
    Loading,
    Picture(Picture),
    Pdf(Box<PdfView>),
    Code(CodeView),
    Document(DocumentView),
    Font(Box<FontView>),
    Folder { items: u64, bytes: Option<u64> },
    Archive(Contents),
    Media,
    Other,
}

impl Shown {
    pub fn from_loaded(loaded: Loaded) -> Self {
        match loaded {
            Loaded::Content(content) => Self::from_content(content),
            Loaded::FontSample(sample) => sample
                .and_then(
                    |Sample {
                         width,
                         height,
                         rgba,
                     }| { Image::from_premultiplied(width, height, rgba) },
                )
                .map_or(Self::Other, |sample| {
                    Self::Font(Box::new(FontView {
                        sample,
                        tinted: None,
                    }))
                }),
            Loaded::Archive(Some(contents)) => Self::Archive(contents),
            Loaded::Archive(None) => Self::Other,
        }
    }

    fn from_content(content: Content) -> Self {
        match content {
            Content::Picture(decoded) => {
                let (levels, frames) = match decoded.pixels {
                    Pixels::Still(levels) => (levels, Vec::new()),
                    Pixels::Animation(frames) => (Vec::new(), frames),
                };
                let next = frames.first().map(|frame| Instant::now() + frame.delay);
                Self::Picture(Picture {
                    width: decoded.width,
                    height: decoded.height,
                    levels,
                    frames,
                    frame: 0,
                    next,
                    shown: None,
                })
            }
            Content::Pdf(path) => match PdfView::open(&path) {
                Some(pdf) => Self::Pdf(Box::new(pdf)),
                None => Self::Other,
            },
            Content::Code(code) => {
                let starts: Vec<usize> = std::iter::once(0)
                    .chain(code.text.match_indices('\n').map(|(at, _)| at + 1))
                    .collect();
                let widest = code
                    .text
                    .lines()
                    .map(|line| line.chars().count())
                    .max()
                    .unwrap_or(0);
                Self::Code(CodeView {
                    code,
                    starts,
                    widest,
                    laid: None,
                })
            }
            Content::Document(blocks) => Self::Document(DocumentView { blocks, laid: None }),
            Content::Folder { items, bytes } => Self::Folder { items, bytes },
            Content::Media => Self::Media,
            Content::Font | Content::Archive | Content::Other => Self::Other,
        }
    }

    pub fn is_picture(&self) -> bool {
        matches!(self, Self::Picture(_))
    }

    pub fn deadline(&self) -> Option<Instant> {
        match self {
            Self::Picture(picture) => picture.next,
            _ => None,
        }
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        let Self::Picture(picture) = self else {
            return false;
        };
        let Some(next) = picture.next.filter(|next| *next <= now) else {
            return false;
        };
        picture.frame = (picture.frame + 1) % picture.frames.len().max(1);
        let delay = picture.frames.get(picture.frame).map(|frame| frame.delay);
        picture.next = delay.map(|delay| (next + delay).max(now));
        true
    }
}
