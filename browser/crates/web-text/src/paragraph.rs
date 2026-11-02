use std::ops::Range;

use icu_segmenter::LineSegmenter;
use icu_segmenter::options::LineBreakOptions;
use unicode_bidi::{BidiInfo, Level};
use web_style::WhiteSpace;

use crate::fonts::{FontId, FontKey, Fonts, is_inherited};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RunStyle {
    pub font: FontKey,
    pub size: f32,
    pub line_height: Option<f32>,
    pub baseline_shift: f32,
    pub white_space: WhiteSpace,
    pub tag: u32,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Glyph {
    pub id: u16,
    y_offset: i16,
    pub x: f32,
}

const Y_UNITS: f32 = 16.0;

impl Glyph {
    pub fn new(id: u16, x: f32, y: f32) -> Self {
        Glyph {
            id,
            y_offset: (y * Y_UNITS).round() as i16,
            x,
        }
    }

    pub fn y(self) -> f32 {
        f32::from(self.y_offset) / Y_UNITS
    }

    pub fn translated(self, x: f32, y: f32) -> (u32, f32, f32) {
        (u32::from(self.id), self.x + x, self.y() + y)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct LineItem {
    pub tag: u32,
    pub font: FontId,
    pub size: f32,
    pub x: f32,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub baseline_shift: f32,
    pub glyphs: Range<usize>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Line {
    pub left: f32,
    pub available: f32,
    pub top: f32,
    pub baseline: f32,
    pub height: f32,
    pub width: f32,
    pub items: Range<usize>,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Lines {
    pub lines: Vec<Line>,
    pub items: Vec<LineItem>,
    pub glyphs: Vec<Glyph>,
    pub height: f32,
}

impl Lines {
    pub fn heap_bytes(&self) -> usize {
        self.lines.capacity() * size_of::<Line>()
            + self.items.capacity() * size_of::<LineItem>()
            + self.glyphs.capacity() * size_of::<Glyph>()
    }
}

#[derive(Default)]
pub struct Paragraph {
    rtl: bool,
    atomics: u16,
    text: String,
    runs: Vec<(u32, RunStyle, Piece)>,
    column: u32,
}

const TAB_COLUMNS: u32 = 8;

impl Paragraph {
    pub fn new(rtl: bool) -> Self {
        Self {
            rtl,
            ..Self::default()
        }
    }

    pub fn is_rtl(&self) -> bool {
        self.rtl
    }

    pub fn is_blank(&self) -> bool {
        self.text.is_empty()
    }

    pub fn push_text(&mut self, text: &str, style: RunStyle) {
        let white_space = style.white_space;
        for ch in text.chars() {
            match ch {
                '\r' => {}
                '\n' if white_space.preserves_newlines() => {
                    self.text.push('\n');
                    self.column = 0;
                }
                ' ' | '\t' | '\n' | '\x0c' if white_space.collapses_spaces() => {
                    if !self.text.is_empty() && !self.text.ends_with([' ', '\n']) {
                        self.text.push(' ');
                    }
                }
                '\t' => {
                    let spaces = TAB_COLUMNS - self.column % TAB_COLUMNS;
                    self.text.extend(std::iter::repeat_n(' ', spaces as usize));
                    self.column += spaces;
                }
                _ => {
                    self.text.push(ch);
                    self.column += 1;
                }
            }
        }
        self.end_run(style);
    }

    pub fn push_break(&mut self, style: RunStyle) {
        self.text.push('\n');
        self.column = 0;
        self.end_run(style);
    }

    fn end_run(&mut self, style: RunStyle) {
        let end = self.text.len() as u32;
        match self.runs.last_mut() {
            Some((last_end, last, Piece::Text)) if *last == style => *last_end = end,
            Some((last_end, _, Piece::Text)) if *last_end == end => {}
            _ => self.runs.push((end, style, Piece::Text)),
        }
    }

    pub fn push_spacer(&mut self, width: f32, style: RunStyle) {
        if width > 0.0 {
            self.runs
                .push((self.text.len() as u32, style, Piece::Spacer(width)));
        }
    }

    pub fn push_atomic(&mut self, index: u16, style: RunStyle) {
        self.runs
            .push((self.text.len() as u32, style, Piece::Atomic(index)));
        self.atomics = self.atomics.max(index + 1);
    }

    pub fn has_atomics(&self) -> bool {
        self.atomics > 0
    }

    fn run_ranges(&self) -> impl Iterator<Item = (Range<usize>, RunStyle, Piece)> + '_ {
        let mut start = 0;
        self.runs.iter().map(move |&(end, style, spacer)| {
            let range = start..end as usize;
            start = end as usize;
            (range, style, spacer)
        })
    }

    fn levels(&self) -> Vec<Level> {
        if !self.rtl && self.text.chars().all(|ch| ch < '\u{590}') {
            return Vec::new();
        }
        let base = if self.rtl { Level::rtl() } else { Level::ltr() };
        BidiInfo::new(&self.text, Some(base)).levels
    }

    pub fn shape(self, fonts: &mut Fonts) -> ShapedParagraph {
        let levels = self.levels();
        let base = if self.rtl { 1 } else { 0 };
        let level_at = |position: usize| {
            levels
                .get(position)
                .map_or(base, |level: &Level| level.number())
        };
        let mut shaped = ShapedParagraph {
            glyphs: Vec::new(),
            spans: Vec::new(),
            styles: self.runs.iter().map(|&(_, style, _)| style).collect(),
        };
        let mut clusters = Vec::new();
        for (run_index, (range, style, piece)) in self.run_ranges().enumerate() {
            if piece != Piece::Text {
                let (flags, id, advance) = match piece {
                    Piece::Atomic(index) => (ATOMIC, index, 0.0),
                    Piece::Spacer(width) => (SPACER, 0, width),
                    Piece::Text => (0, 0, 0.0),
                };
                if let Some(font) = fonts.primary(style.font) {
                    shaped.spans.push(Span {
                        font,
                        run: run_index as u32,
                        level: level_at(range.start.saturating_sub(1)),
                    });
                    shaped.glyphs.push(ShapedGlyph {
                        id,
                        flags,
                        span: (shaped.spans.len() - 1) as u32,
                        advance,
                        x_offset: 0.0,
                        y_offset: 0.0,
                    });
                    clusters.push(range.start as u32);
                }
                continue;
            }
            let Some(run_text) = self.text.get(range.clone()) else {
                continue;
            };
            let mut group: Option<(Span, usize)> = None;
            let mut previous_font = None;
            let mut ascii = [None; 128];
            for (offset, ch) in run_text.char_indices() {
                let position = range.start + offset;
                let font = match ascii.get_mut(ch as usize) {
                    _ if ch == '\n' => None,
                    Some(Some(font)) => *font,
                    Some(slot) => *slot.insert(fonts.for_char(style.font, ch)),
                    None if is_inherited(ch) && previous_font.is_some() => previous_font,
                    None => fonts.for_char(style.font, ch),
                };
                previous_font = font.or(previous_font);
                let span = font.map(|font| Span {
                    font,
                    run: run_index as u32,
                    level: level_at(position),
                });
                if group.map(|(group_span, _)| group_span) != span {
                    if let Some((group_span, start)) = group.take() {
                        self.shape_group(
                            fonts,
                            group_span,
                            start..position,
                            style.size,
                            &mut shaped,
                            &mut clusters,
                        );
                    }
                    group = span.map(|span| (span, position));
                }
                if ch == '\n'
                    && let Some(font) = previous_font.or_else(|| fonts.primary(style.font))
                {
                    shaped.spans.push(Span {
                        font,
                        run: run_index as u32,
                        level: base,
                    });
                    shaped.glyphs.push(ShapedGlyph {
                        id: 0,
                        flags: NEWLINE | MANDATORY,
                        span: (shaped.spans.len() - 1) as u32,
                        advance: 0.0,
                        x_offset: 0.0,
                        y_offset: 0.0,
                    });
                    clusters.push(position as u32);
                }
            }
            if let Some((group_span, start)) = group {
                self.shape_group(
                    fonts,
                    group_span,
                    start..range.end,
                    style.size,
                    &mut shaped,
                    &mut clusters,
                );
            }
        }
        self.mark_breaks(&mut shaped.glyphs, &clusters);
        shaped
    }

    fn mark_breaks(&self, glyphs: &mut [ShapedGlyph], clusters: &[u32]) {
        let text = self.text.as_str();
        let bytes = text.as_bytes();
        for (glyph, &cluster) in glyphs.iter_mut().zip(clusters) {
            if glyph.flags & (SPACER | ATOMIC) == 0 && bytes.get(cluster as usize) == Some(&b' ') {
                glyph.flags |= SPACE;
            }
        }
        let wraps_before = |position: usize| {
            self.runs
                .iter()
                .find(|&&(end, _, piece)| end as usize >= position && piece == Piece::Text)
                .is_none_or(|(_, style, _)| style.white_space.wraps())
        };
        let mut glyph = 0;
        let breaks = LineSegmenter::new_auto(LineBreakOptions::default()).segment_str(text);
        for position in breaks {
            if position == 0 || !(bytes.get(position - 1) == Some(&b'\n') || wraps_before(position))
            {
                continue;
            }
            while clusters
                .get(glyph + 1)
                .is_some_and(|&cluster| (cluster as usize) < position)
            {
                glyph += 1;
            }
            if let Some(shaped) = glyphs.get_mut(glyph)
                && clusters
                    .get(glyph)
                    .is_some_and(|&cluster| (cluster as usize) < position)
            {
                shaped.flags |= BREAK;
            }
        }
    }

    fn shape_group(
        &self,
        fonts: &mut Fonts,
        span: Span,
        range: Range<usize>,
        size: f32,
        shaped: &mut ShapedParagraph,
        clusters: &mut Vec<u32>,
    ) {
        let Some(text) = self.text.get(range.clone()) else {
            return;
        };
        let rtl = span.level % 2 == 1;
        let span_index = shaped.spans.len() as u32;
        shaped.spans.push(span);
        let mut offset = range.start as u32;
        for word in text.split_inclusive(' ') {
            if let Some(glyphs) = fonts.shape_word(span.font, size, rtl, word) {
                for glyph in glyphs {
                    shaped.glyphs.push(ShapedGlyph {
                        id: u16::try_from(glyph.id).unwrap_or(0),
                        flags: 0,
                        span: span_index,
                        advance: glyph.advance,
                        x_offset: glyph.x_offset,
                        y_offset: glyph.y_offset,
                    });
                    clusters.push(offset + glyph.cluster);
                }
            }
            offset += word.len() as u32;
        }
    }
}

const SPACE: u8 = 1;
const BREAK: u8 = 2;
const MANDATORY: u8 = 4;
const NEWLINE: u8 = 8;
const SPACER: u8 = 16;
const ATOMIC: u8 = 32;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Piece {
    Text,
    Spacer(f32),
    Atomic(u16),
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct AtomicSize {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Span {
    font: FontId,
    run: u32,
    level: u8,
}

#[derive(Clone, Copy, Debug)]
struct ShapedGlyph {
    id: u16,
    flags: u8,
    span: u32,
    advance: f32,
    x_offset: f32,
    y_offset: f32,
}

impl ShapedGlyph {
    fn hangs(&self) -> bool {
        self.flags & (SPACE | NEWLINE) != 0
    }
}

pub struct ShapedParagraph {
    glyphs: Vec<ShapedGlyph>,
    spans: Vec<Span>,
    styles: Vec<RunStyle>,
}

struct Segment {
    glyphs: Range<usize>,
    width: f32,
    trailing: f32,
    mandatory: bool,
}

impl ShapedParagraph {
    pub fn heap_bytes(&self) -> usize {
        self.glyphs.capacity() * size_of::<ShapedGlyph>()
            + self.spans.capacity() * size_of::<Span>()
            + self.styles.capacity() * size_of::<RunStyle>()
    }

    pub fn intrinsic(&self, atomics: &[AtomicSize]) -> (f32, f32) {
        let mut minimum: f32 = 0.0;
        let mut maximum: f32 = 0.0;
        let mut line = 0.0;
        for segment in self.segments(atomics) {
            minimum = minimum.max(segment.width - segment.trailing);
            line += segment.width;
            if segment.mandatory {
                maximum = maximum.max(line - segment.trailing);
                line = 0.0;
            }
        }
        (minimum, maximum.max(line))
    }

    pub fn lines(
        &self,
        fonts: &mut Fonts,
        space: &mut dyn FnMut(f32) -> (f32, f32),
        strut: RunStyle,
        justify: bool,
        atomics: &[AtomicSize],
    ) -> Lines {
        let mut lines = Lines::default();
        let mut start = 0;
        let mut line_width = 0.0;
        let mut has_content = false;
        let (mut left, mut width) = space(0.0);
        for segment in self.segments(atomics) {
            if has_content && line_width + segment.width - segment.trailing > width {
                let justify_to = justify.then_some(width);
                self.push_line(
                    &mut lines,
                    fonts,
                    start..segment.glyphs.start,
                    strut,
                    justify_to,
                    (left, width),
                    atomics,
                );
                (left, width) = space(lines.height);
                start = segment.glyphs.start;
                line_width = 0.0;
            }
            line_width += segment.width;
            has_content = true;
            if segment.mandatory {
                self.push_line(
                    &mut lines,
                    fonts,
                    start..segment.glyphs.end,
                    strut,
                    None,
                    (left, width),
                    atomics,
                );
                (left, width) = space(lines.height);
                start = segment.glyphs.end;
                line_width = 0.0;
                has_content = false;
            }
        }
        if has_content && start < self.glyphs.len() {
            self.push_line(
                &mut lines,
                fonts,
                start..self.glyphs.len(),
                strut,
                None,
                (left, width),
                atomics,
            );
        }
        lines
    }

    fn advance(glyph: &ShapedGlyph, atomics: &[AtomicSize]) -> f32 {
        if glyph.flags & ATOMIC != 0 {
            atomics
                .get(usize::from(glyph.id))
                .map_or(0.0, |atomic| atomic.width)
        } else {
            glyph.advance
        }
    }

    fn segments<'a>(&'a self, atomics: &'a [AtomicSize]) -> impl Iterator<Item = Segment> + 'a {
        let mut start = 0;
        std::iter::from_fn(move || {
            let first = start;
            let mut width = 0.0;
            let mut trailing = 0.0;
            while let Some(glyph) = self.glyphs.get(start) {
                start += 1;
                let advance = Self::advance(glyph, atomics);
                width += advance;
                if glyph.hangs() {
                    trailing += advance;
                } else {
                    trailing = 0.0;
                }
                if glyph.flags & BREAK != 0 {
                    return Some(Segment {
                        glyphs: first..start,
                        width,
                        trailing,
                        mandatory: glyph.flags & MANDATORY != 0,
                    });
                }
            }
            (start > first).then_some(Segment {
                glyphs: first..start,
                width,
                trailing,
                mandatory: false,
            })
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn push_line(
        &self,
        lines: &mut Lines,
        fonts: &mut Fonts,
        range: Range<usize>,
        strut: RunStyle,
        justify_to: Option<f32>,
        left: (f32, f32),
        atomics: &[AtomicSize],
    ) {
        let (mut above, mut below) = vertical_extent(fonts, None, strut);
        let first_item = lines.items.len();
        let shaped = self.glyphs.get(range).unwrap_or_default();
        let hanging = shaped
            .iter()
            .rev()
            .take_while(|glyph| glyph.hangs())
            .count();
        let shaped = shaped.get(..shaped.len() - hanging).unwrap_or_default();
        let order = visual_order(shaped, |glyph| {
            self.spans
                .get(glyph.span as usize)
                .map_or(0, |span| span.level)
        });
        let natural: f32 = shaped
            .iter()
            .map(|glyph| Self::advance(glyph, atomics))
            .sum();
        let spaces = shaped
            .iter()
            .filter(|glyph| glyph.flags & SPACE != 0)
            .count();
        let expansion = match justify_to {
            Some(width) if spaces > 0 && width > natural => (width - natural) / spaces as f32,
            _ => 0.0,
        };
        let mut pen = 0.0;
        let visual: Vec<&ShapedGlyph> = order
            .iter()
            .filter_map(|&index| shaped.get(index))
            .filter(|glyph| glyph.flags & NEWLINE == 0)
            .collect();
        for group in visual.chunk_by(|a, b| a.span == b.span) {
            let Some((span, style)) = group.first().and_then(|first| {
                let span = *self.spans.get(first.span as usize)?;
                Some((span, *self.styles.get(span.run as usize)?))
            }) else {
                continue;
            };
            let atomic = group
                .first()
                .filter(|glyph| glyph.flags & ATOMIC != 0)
                .and_then(|glyph| atomics.get(usize::from(glyph.id)));
            let (item_above, item_below) = match atomic {
                Some(atomic) => (
                    atomic.ascent + style.baseline_shift,
                    atomic.descent - style.baseline_shift,
                ),
                None => vertical_extent(fonts, Some(span.font), style),
            };
            above = f32::max(above, item_above);
            below = f32::max(below, item_below);
            let metrics = fonts.metrics(span.font);
            let scale = style.size / metrics.units_per_em;
            let start = pen;
            let glyph_start = lines.glyphs.len();
            for glyph in group {
                if glyph.flags & (SPACER | ATOMIC) == 0 {
                    lines.glyphs.push(Glyph::new(
                        glyph.id,
                        pen - start + glyph.x_offset,
                        -glyph.y_offset,
                    ));
                }
                pen += Self::advance(glyph, atomics);
                if glyph.flags & SPACE != 0 {
                    pen += expansion;
                }
            }
            lines.items.push(LineItem {
                tag: style.tag,
                font: span.font,
                size: style.size,
                x: start,
                width: pen - start,
                ascent: atomic.map_or(metrics.ascent * scale, |atomic| atomic.ascent),
                descent: atomic.map_or(metrics.descent * scale, |atomic| atomic.descent),
                baseline_shift: style.baseline_shift,
                glyphs: glyph_start..lines.glyphs.len(),
            });
        }
        let top = lines.height;
        lines.lines.push(Line {
            left: left.0,
            available: left.1,
            top,
            baseline: top + above,
            height: above + below,
            width: pen,
            items: first_item..lines.items.len(),
        });
        lines.height = top + above + below;
    }
}

fn visual_order(glyphs: &[ShapedGlyph], level: impl Fn(&ShapedGlyph) -> u8) -> Vec<usize> {
    let mut order: Vec<usize> = (0..glyphs.len()).collect();
    let levels: Vec<u8> = glyphs.iter().map(&level).collect();
    let highest = levels.iter().copied().max().unwrap_or(0);
    let Some(lowest_odd) = levels.iter().copied().filter(|level| level % 2 == 1).min() else {
        return order;
    };
    for threshold in (lowest_odd..=highest).rev() {
        let mut index = 0;
        while index < order.len() {
            let at = |index: usize| {
                order
                    .get(index)
                    .and_then(|&glyph| levels.get(glyph))
                    .copied()
                    .unwrap_or(0)
            };
            if at(index) >= threshold {
                let mut end = index;
                while end < order.len() && at(end) >= threshold {
                    end += 1;
                }
                if let Some(run) = order.get_mut(index..end) {
                    run.reverse();
                }
                index = end;
            } else {
                index += 1;
            }
        }
    }
    order
}

fn vertical_extent(fonts: &mut Fonts, font: Option<FontId>, style: RunStyle) -> (f32, f32) {
    let Some(font) = font.or_else(|| fonts.primary(style.font)) else {
        let height = style.line_height.unwrap_or(style.size * 1.2);
        return (height * 0.8, height * 0.2);
    };
    let metrics = fonts.metrics(font);
    let scale = style.size / metrics.units_per_em;
    let ascent = metrics.ascent * scale;
    let descent = metrics.descent * scale;
    let line_height = style
        .line_height
        .unwrap_or(ascent + descent + metrics.line_gap * scale);
    let half_leading = (line_height - ascent - descent) / 2.0;
    (
        ascent + half_leading + style.baseline_shift,
        descent + half_leading - style.baseline_shift,
    )
}
