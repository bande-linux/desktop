use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use bande_design::{Color, TextRole, TextSize, Typeface};
use parley::fontique::GenericFamily;
use parley::{
    Affinity, Alignment, AlignmentOptions, Cursor, FontContext, FontData, FontFamily,
    FontFamilyName, FontWeight, Layout, LayoutContext, LineHeight, PositionedLayoutItem,
    RangedBuilder, Selection, StyleProperty,
};

use crate::Rect;

const DISPLAY_SIZE: f32 = 20.0;
const ELLIPSIS: char = '\u{2026}';
const KEPT_TAIL: usize = 7;
const CACHED_PARAGRAPHS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Cut {
    #[default]
    End,
    Middle,
}

pub fn shortened(text: &str, cut: Cut, mut fits: impl FnMut(&str) -> bool) -> String {
    if fits(text) {
        return text.to_owned();
    }
    let characters = text.chars().collect::<Vec<_>>();
    let tail = match cut {
        Cut::End => 0,
        Cut::Middle => KEPT_TAIL.min(characters.len() / 3),
    };
    let tail_from = characters.len() - tail;
    let candidate = |kept: usize| {
        characters[..kept]
            .iter()
            .collect::<String>()
            .trim_end()
            .chars()
            .chain([ELLIPSIS])
            .chain(characters[tail_from..].iter().copied())
            .collect::<String>()
    };
    let (mut low, mut high) = (0, tail_from);
    while low < high {
        let middle = (low + high).div_ceil(2);
        if fits(&candidate(middle)) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    candidate(low)
}

static INTERFACE_FAMILIES: [FontFamilyName<'static>; 2] = [
    FontFamilyName::named(Typeface::Interface.family()),
    FontFamilyName::Generic(GenericFamily::SansSerif),
];

static DISPLAY_FAMILIES: [FontFamilyName<'static>; 3] = [
    FontFamilyName::named(Typeface::Display.family()),
    FontFamilyName::named(Typeface::Interface.family()),
    FontFamilyName::Generic(GenericFamily::SansSerif),
];

static MONOSPACE_FAMILIES: [FontFamilyName<'static>; 3] = [
    FontFamilyName::named(Typeface::Monospace.family()),
    FontFamilyName::named("DejaVu Sans Mono"),
    FontFamilyName::Generic(GenericFamily::Monospace),
];

static EMOJI_FAMILIES: [FontFamilyName<'static>; 3] = [
    FontFamilyName::named(Typeface::Emoji.family()),
    FontFamilyName::named(Typeface::Interface.family()),
    FontFamilyName::Generic(GenericFamily::SansSerif),
];

fn families(typeface: Typeface) -> FontFamily<'static> {
    let families = match typeface {
        Typeface::Interface => INTERFACE_FAMILIES.as_slice(),
        Typeface::Display => DISPLAY_FAMILIES.as_slice(),
        Typeface::Monospace => MONOSPACE_FAMILIES.as_slice(),
        Typeface::Emoji => EMOJI_FAMILIES.as_slice(),
    };
    FontFamily::List(families.into())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub weight: f32,
    pub color: Color,
    pub typeface: Typeface,
    pub tabular: bool,
    pub line_height: Option<f32>,
    pub tracking: f32,
    pub caps: bool,
}

impl TextStyle {
    pub const REGULAR: f32 = 400.0;
    pub const MEDIUM: f32 = 500.0;
    pub const SEMIBOLD: f32 = 600.0;
    pub const BOLD: f32 = 700.0;

    pub const fn new(size: f32, color: Color) -> Self {
        Self {
            size,
            weight: Self::REGULAR,
            color,
            typeface: if size >= DISPLAY_SIZE {
                Typeface::Display
            } else {
                Typeface::Interface
            },
            tabular: false,
            line_height: None,
            tracking: 0.0,
            caps: false,
        }
    }

    pub const fn role(role: TextRole, color: Color) -> Self {
        let spec = role.spec();
        Self {
            size: spec.size,
            weight: spec.weight,
            color,
            typeface: spec.typeface,
            tabular: false,
            line_height: Some(spec.line_height),
            tracking: spec.tracking,
            caps: spec.caps,
        }
    }

    pub const fn tabular(self) -> Self {
        Self {
            tabular: true,
            ..self
        }
    }

    pub const fn monospace(self) -> Self {
        Self {
            typeface: Typeface::Monospace,
            ..self
        }
    }

    pub const fn emoji(self) -> Self {
        Self {
            typeface: Typeface::Emoji,
            ..self
        }
    }

    pub const fn weight(self, weight: f32) -> Self {
        Self { weight, ..self }
    }

    pub fn apply(&self, builder: &mut RangedBuilder<'_, Color>) {
        builder.push_default(StyleProperty::FontFamily(families(self.typeface)));
        builder.push_default(StyleProperty::FontSize(self.size));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(self.weight)));
        builder.push_default(StyleProperty::Brush(self.color));
        builder.push_default(StyleProperty::FontFeatures(
            self.typeface.features(self.tabular).into(),
        ));
        if let Some(line_height) = self.line_height {
            builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(line_height)));
        }
        if self.tracking != 0.0 {
            builder.push_default(StyleProperty::LetterSpacing(self.tracking * self.size));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
}

#[derive(PartialEq, Eq, Hash)]
struct ParagraphKey {
    text: String,
    size: u32,
    weight: u32,
    color: Color,
    typeface: Typeface,
    tabular: bool,
    line_height: Option<u32>,
    tracking: u32,
    caps: bool,
    emphasis: Vec<(usize, usize, u32)>,
    max_width: Option<u32>,
    align: TextAlign,
}

impl ParagraphKey {
    fn new(
        text: &str,
        style: TextStyle,
        emphasis: &[(Range<usize>, f32)],
        max_width: Option<f32>,
        align: TextAlign,
    ) -> Self {
        Self {
            text: text.to_owned(),
            size: style.size.to_bits(),
            weight: style.weight.to_bits(),
            color: style.color,
            typeface: style.typeface,
            tabular: style.tabular,
            line_height: style.line_height.map(f32::to_bits),
            tracking: style.tracking.to_bits(),
            caps: style.caps,
            emphasis: emphasis
                .iter()
                .map(|(range, weight)| (range.start, range.end, weight.to_bits()))
                .collect(),
            max_width: max_width.map(f32::to_bits),
            align,
        }
    }
}

#[derive(Default)]
struct ParagraphCache {
    recent: HashMap<ParagraphKey, Paragraph>,
    older: HashMap<ParagraphKey, Paragraph>,
}

impl ParagraphCache {
    fn get(&mut self, key: ParagraphKey) -> Result<Paragraph, ParagraphKey> {
        if let Some(paragraph) = self.recent.get(&key) {
            return Ok(paragraph.clone());
        }
        match self.older.remove(&key) {
            Some(paragraph) => {
                self.insert(key, paragraph.clone());
                Ok(paragraph)
            }
            None => Err(key),
        }
    }

    fn insert(&mut self, key: ParagraphKey, paragraph: Paragraph) {
        if self.recent.len() >= CACHED_PARAGRAPHS {
            self.older = std::mem::take(&mut self.recent);
        }
        self.recent.insert(key, paragraph);
    }

    fn clear(&mut self) {
        self.recent.clear();
        self.older.clear();
    }
}

pub struct TextSystem {
    fonts: FontContext,
    layouts: LayoutContext<Color>,
    size: TextSize,
    monospace: Option<Arc<str>>,
    cache: ParagraphCache,
}

impl Default for TextSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl TextSystem {
    pub fn new() -> Self {
        Self {
            fonts: FontContext::new(),
            layouts: LayoutContext::new(),
            size: TextSize::DEFAULT,
            monospace: None,
            cache: ParagraphCache::default(),
        }
    }

    pub fn sharing_fonts(&self) -> Self {
        Self {
            fonts: self.fonts.clone(),
            layouts: LayoutContext::new(),
            size: self.size,
            monospace: self.monospace.clone(),
            cache: ParagraphCache::default(),
        }
    }

    pub fn set_monospace_family(&mut self, family: Option<&str>) {
        self.monospace = family.map(Arc::from);
        self.cache.clear();
    }

    pub fn has_family(&mut self, family: &str) -> bool {
        self.fonts.collection.family_id(family).is_some()
    }

    pub fn set_size(&mut self, size: TextSize) {
        if self.size != size {
            self.size = size;
            self.cache.clear();
        }
    }

    pub fn size(&self) -> TextSize {
        self.size
    }

    pub fn scale(&self) -> f32 {
        self.size.scale()
    }

    pub fn contexts(&mut self) -> (&mut FontContext, &mut LayoutContext<Color>) {
        (&mut self.fonts, &mut self.layouts)
    }

    pub fn paragraph(
        &mut self,
        text: &str,
        style: TextStyle,
        max_width: Option<f32>,
        align: TextAlign,
    ) -> Paragraph {
        self.emphasized(text, style, &[], max_width, align)
    }

    pub fn fitted(&mut self, text: &str, style: TextStyle, max_width: f32) -> Paragraph {
        let paragraph = self.paragraph(text, style, None, TextAlign::Start);
        if paragraph.width() <= max_width {
            return paragraph;
        }
        let shown = shortened(text, Cut::End, |candidate| {
            self.paragraph(candidate, style, None, TextAlign::Start)
                .width()
                <= max_width
        });
        self.paragraph(&shown, style, None, TextAlign::Start)
    }

    pub fn emphasized(
        &mut self,
        text: &str,
        style: TextStyle,
        emphasis: &[(Range<usize>, f32)],
        max_width: Option<f32>,
        align: TextAlign,
    ) -> Paragraph {
        let key = match self
            .cache
            .get(ParagraphKey::new(text, style, emphasis, max_width, align))
        {
            Ok(paragraph) => return paragraph,
            Err(key) => key,
        };
        let paragraph = self.shape(text, style, emphasis, max_width, align);
        self.cache.insert(key, paragraph.clone());
        paragraph
    }

    fn shape(
        &mut self,
        text: &str,
        style: TextStyle,
        emphasis: &[(Range<usize>, f32)],
        max_width: Option<f32>,
        align: TextAlign,
    ) -> Paragraph {
        let text = if style.caps {
            Cow::Owned(text.to_uppercase())
        } else {
            Cow::Borrowed(text)
        };
        let mut builder =
            self.layouts
                .ranged_builder(&mut self.fonts, &text, self.size.scale(), true);
        style.apply(&mut builder);
        if let (Typeface::Monospace, Some(family)) = (style.typeface, &self.monospace) {
            let families = std::iter::once(FontFamilyName::named(family))
                .chain(MONOSPACE_FAMILIES.iter().cloned())
                .collect::<Vec<_>>();
            builder.push_default(StyleProperty::FontFamily(FontFamily::List(families.into())));
        }
        for (range, weight) in emphasis {
            if text.get(range.clone()).is_some() {
                builder.push(
                    StyleProperty::FontWeight(FontWeight::new(*weight)),
                    range.clone(),
                );
            }
        }
        let mut layout = builder.build(&text);
        layout.break_all_lines(max_width);
        let alignment = match align {
            TextAlign::Start => Alignment::Start,
            TextAlign::Center => Alignment::Center,
            TextAlign::End => Alignment::End,
        };
        layout.align(alignment, AlignmentOptions::default());
        Paragraph {
            layout: Arc::new(layout),
        }
    }
}

#[derive(Clone)]
pub struct Paragraph {
    layout: Arc<Layout<Color>>,
}

impl fmt::Debug for Paragraph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Paragraph")
            .field("width", &self.width())
            .field("height", &self.height())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedGlyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone)]
pub struct GlyphRun {
    pub(crate) font: FontData,
    pub size: f32,
    pub color: Color,
    pub skew_degrees: Option<f32>,
    pub(crate) coords: Vec<i16>,
    pub glyphs: Vec<PositionedGlyph>,
}

impl PartialEq for GlyphRun {
    fn eq(&self, other: &Self) -> bool {
        self.font_blob_id() == other.font_blob_id()
            && self.font.index == other.font.index
            && self.size == other.size
            && self.color == other.color
            && self.skew_degrees == other.skew_degrees
            && self.coords == other.coords
            && self.glyphs == other.glyphs
    }
}

impl PartialEq for Paragraph {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.layout, &other.layout) || self.glyph_runs() == other.glyph_runs()
    }
}

impl GlyphRun {
    pub fn font_bytes(&self) -> &[u8] {
        self.font.data.data()
    }

    pub fn font_index(&self) -> u32 {
        self.font.index
    }

    pub(crate) fn font_blob_id(&self) -> u64 {
        self.font.data.id()
    }

    pub fn variation_coords(&self) -> &[i16] {
        &self.coords
    }
}

impl Paragraph {
    pub fn from_layout(layout: Layout<Color>) -> Self {
        Self {
            layout: Arc::new(layout),
        }
    }

    pub fn layout(&self) -> &Layout<Color> {
        &self.layout
    }

    pub fn width(&self) -> f32 {
        self.layout.width()
    }

    pub fn height(&self) -> f32 {
        self.layout.height()
    }

    pub fn line_count(&self) -> usize {
        self.layout.len()
    }

    pub fn span_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let mut rects = Vec::new();
        Selection::new(
            Cursor::from_byte_index(&self.layout, range.start, Affinity::Downstream),
            Cursor::from_byte_index(&self.layout, range.end, Affinity::Upstream),
        )
        .geometry_with(&self.layout, |rect, _| {
            rects.push(Rect::new(
                rect.x0 as f32,
                rect.y0 as f32,
                (rect.x1 - rect.x0) as f32,
                (rect.y1 - rect.y0) as f32,
            ));
        });
        rects
    }

    pub fn glyph_runs(&self) -> Vec<GlyphRun> {
        let mut runs = Vec::new();
        for line in self.layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                let run = glyph_run.run();
                runs.push(GlyphRun {
                    font: run.font().clone(),
                    size: run.font_size(),
                    color: glyph_run.style().brush,
                    skew_degrees: run.synthesis().skew(),
                    coords: run
                        .normalized_coords()
                        .iter()
                        .map(|coord| coord.to_bits())
                        .collect(),
                    glyphs: glyph_run
                        .positioned_glyphs()
                        .map(|glyph| PositionedGlyph {
                            id: glyph.id,
                            x: glyph.x,
                            y: glyph.y,
                        })
                        .collect(),
                });
            }
        }
        runs
    }
}
