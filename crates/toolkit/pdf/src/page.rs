use std::ops::Range;

use bande_graphics::{Item, Paragraph, Point, Rect};

const POINTS_PER_MILLIMETRE: f32 = 72.0 / 25.4;
const DEFAULT_MARGIN: f32 = 54.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Paper {
    pub keyword: &'static str,
    pub width: f32,
    pub height: f32,
}

impl Paper {
    const fn millimetres(keyword: &'static str, width: f32, height: f32) -> Self {
        Self {
            keyword,
            width: width * POINTS_PER_MILLIMETRE,
            height: height * POINTS_PER_MILLIMETRE,
        }
    }

    pub const A3: Self = Self::millimetres("iso_a3_297x420mm", 297.0, 420.0);
    pub const A4: Self = Self::millimetres("iso_a4_210x297mm", 210.0, 297.0);
    pub const A5: Self = Self::millimetres("iso_a5_148x210mm", 148.0, 210.0);
    pub const LETTER: Self = Self {
        keyword: "na_letter_8.5x11in",
        width: 612.0,
        height: 792.0,
    };
    pub const LEGAL: Self = Self {
        keyword: "na_legal_8.5x14in",
        width: 612.0,
        height: 1008.0,
    };
    pub const ALL: [Self; 5] = [Self::A4, Self::LETTER, Self::A3, Self::A5, Self::LEGAL];

    pub fn with_keyword(keyword: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|paper| paper.keyword == keyword)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSetup {
    pub paper: Paper,
    pub orientation: Orientation,
    pub margin: f32,
}

impl Default for PageSetup {
    fn default() -> Self {
        Self {
            paper: Paper::A4,
            orientation: Orientation::Portrait,
            margin: DEFAULT_MARGIN,
        }
    }
}

impl PageSetup {
    pub fn size(&self) -> (f32, f32) {
        match self.orientation {
            Orientation::Portrait => (self.paper.width, self.paper.height),
            Orientation::Landscape => (self.paper.height, self.paper.width),
        }
    }

    pub fn content(&self) -> Rect {
        let (width, height) = self.size();
        Rect::new(
            self.margin,
            self.margin,
            (width - 2.0 * self.margin).max(1.0),
            (height - 2.0 * self.margin).max(1.0),
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PageItem {
    Scene(Item),
    Lines {
        origin: Point,
        paragraph: Paragraph,
        lines: Range<usize>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub size: (f32, f32),
    pub items: Vec<PageItem>,
}

impl Page {
    pub fn new(setup: &PageSetup) -> Self {
        Self {
            size: setup.size(),
            items: Vec::new(),
        }
    }

    pub fn draw(&mut self, item: Item) {
        self.items.push(PageItem::Scene(item));
    }
}
