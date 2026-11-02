use std::ops::Range;

use harfrust::{BufferClusterLevel, Direction, ShapeOptions, UnicodeBuffer};
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
    pub id: u32,
    pub x: f32,
    pub y: f32,
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
    text: String,
    runs: Vec<(u32, RunStyle)>,
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

    pub fn text(&self) -> &str {
        &self.text
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
            Some((last_end, last)) if *last == style => *last_end = end,
            Some((last_end, _)) if *last_end == end => {}
            _ => self.runs.push((end, style)),
        }
    }

    fn run_ranges(&self) -> impl Iterator<Item = (Range<usize>, RunStyle)> + '_ {
        let mut start = 0;
        self.runs.iter().map(move |&(end, style)| {
            let range = start..end as usize;
            start = end as usize;
            (range, style)
        })
    }

    fn levels(&self) -> Vec<Level> {
        if !self.rtl && self.text.chars().all(|ch| ch < '\u{590}') {
            return Vec::new();
        }
        let base = if self.rtl { Level::rtl() } else { Level::ltr() };
        BidiInfo::new(&self.text, Some(base)).levels
    }

    pub fn shape(&self, fonts: &mut Fonts) -> ShapedParagraph {
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
        };
        let mut buffer = UnicodeBuffer::new();
        for (run_index, (range, style)) in self.run_ranges().enumerate() {
            let Some(run_text) = self.text.get(range.clone()) else {
                continue;
            };
            let mut group: Option<(Span, usize)> = None;
            let mut previous_font = None;
            for (offset, ch) in run_text.char_indices() {
                let position = range.start + offset;
                let font = if ch == '\n' {
                    None
                } else if is_inherited(ch) && previous_font.is_some() {
                    previous_font
                } else {
                    fonts.for_char(style.font, ch)
                };
                previous_font = font.or(previous_font);
                let span = font.map(|font| Span {
                    font,
                    run: run_index as u32,
                    level: level_at(position),
                });
                if group.map(|(group_span, _)| group_span) != span {
                    if let Some((group_span, start)) = group.take() {
                        buffer = self.shape_group(
                            fonts,
                            group_span,
                            start..position,
                            style.size,
                            buffer,
                            &mut shaped,
                        );
                    }
                    group = span.map(|span| (span, position));
                }
            }
            if let Some((group_span, start)) = group {
                buffer = self.shape_group(
                    fonts,
                    group_span,
                    start..range.end,
                    style.size,
                    buffer,
                    &mut shaped,
                );
            }
        }
        shaped
    }

    fn shape_group(
        &self,
        fonts: &Fonts,
        span: Span,
        range: Range<usize>,
        size: f32,
        mut buffer: UnicodeBuffer,
        shaped: &mut ShapedParagraph,
    ) -> UnicodeBuffer {
        let (Some(loaded), Some(text)) = (fonts.loaded(span.font), self.text.get(range.clone()))
        else {
            return buffer;
        };
        let Ok(font_ref) =
            harfrust::FontRef::from_index(loaded.data.data.data(), loaded.data.index)
        else {
            return buffer;
        };
        let shaper = loaded.shaper.shaper(&font_ref).build();
        for (offset, ch) in text.char_indices() {
            buffer.add(ch, (range.start + offset) as u32);
        }
        buffer.set_pre_context(self.text.get(..range.start).unwrap_or_default());
        buffer.set_post_context(self.text.get(range.end..).unwrap_or_default());
        buffer.guess_segment_properties();
        let rtl = span.level % 2 == 1;
        buffer.set_direction(if rtl {
            Direction::RightToLeft
        } else {
            Direction::LeftToRight
        });
        buffer.set_cluster_level(BufferClusterLevel::MonotoneCharacters);
        let output = shaper.shape(buffer, ShapeOptions::new().point_size(Some(size)));
        let scale = size / loaded.metrics.units_per_em;
        let span_index = shaped.spans.len() as u32;
        shaped.spans.push(span);
        let first = shaped.glyphs.len();
        shaped.glyphs.extend(
            output
                .glyph_infos()
                .iter()
                .zip(output.glyph_positions())
                .map(|(info, position)| ShapedGlyph {
                    id: info.glyph_id,
                    cluster: info.cluster,
                    advance: position.x_advance as f32 * scale,
                    x_offset: position.x_offset as f32 * scale,
                    y_offset: position.y_offset as f32 * scale,
                    span: span_index,
                }),
        );
        if rtl && let Some(glyphs) = shaped.glyphs.get_mut(first..) {
            glyphs.reverse();
        }
        output.clear()
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Span {
    font: FontId,
    run: u32,
    level: u8,
}

#[derive(Clone, Copy, Debug)]
struct ShapedGlyph {
    id: u32,
    cluster: u32,
    advance: f32,
    x_offset: f32,
    y_offset: f32,
    span: u32,
}

pub struct ShapedParagraph {
    glyphs: Vec<ShapedGlyph>,
    spans: Vec<Span>,
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
    }

    pub fn lines(
        &self,
        paragraph: &Paragraph,
        fonts: &mut Fonts,
        width: f32,
        strut: RunStyle,
        justify: bool,
    ) -> Lines {
        let justify_to = justify.then_some(width);
        let styles: Vec<RunStyle> = paragraph.runs.iter().map(|&(_, style)| style).collect();
        let text = paragraph.text.as_bytes();
        let segments = self.segments(paragraph);
        let mut lines = Lines::default();
        let mut start = 0;
        let mut line_width = 0.0;
        let mut has_content = false;
        for segment in &segments {
            if has_content && line_width + segment.width - segment.trailing > width {
                self.push_line(
                    &mut lines,
                    fonts,
                    &styles,
                    text,
                    start..segment.glyphs.start,
                    strut,
                    justify_to,
                );
                start = segment.glyphs.start;
                line_width = 0.0;
            }
            line_width += segment.width;
            has_content = true;
            if segment.mandatory {
                self.push_line(
                    &mut lines,
                    fonts,
                    &styles,
                    text,
                    start..segment.glyphs.end,
                    strut,
                    None,
                );
                start = segment.glyphs.end;
                line_width = 0.0;
                has_content = false;
            }
        }
        if has_content && start < self.glyphs.len() {
            self.push_line(
                &mut lines,
                fonts,
                &styles,
                text,
                start..self.glyphs.len(),
                strut,
                None,
            );
        }
        lines
    }

    fn segments(&self, paragraph: &Paragraph) -> Vec<Segment> {
        let text = paragraph.text.as_str();
        let bytes = text.as_bytes();
        let wraps_before = |position: usize| {
            paragraph
                .runs
                .iter()
                .find(|&&(end, _)| end as usize >= position)
                .is_none_or(|(_, style)| style.white_space.wraps())
        };
        let breaks = LineSegmenter::new_auto(LineBreakOptions::default())
            .segment_str(text)
            .filter(|&position| {
                position > 0
                    && (position == text.len()
                        || bytes.get(position - 1) == Some(&b'\n')
                        || wraps_before(position))
            });
        let mut segments = Vec::new();
        let mut glyph = 0;
        for end in breaks {
            let first = glyph;
            let mut width = 0.0;
            let mut trailing = 0.0;
            while let Some(shaped) = self.glyphs.get(glyph)
                && (shaped.cluster as usize) < end
            {
                width += shaped.advance;
                if bytes.get(shaped.cluster as usize) == Some(&b' ') {
                    trailing += shaped.advance;
                } else {
                    trailing = 0.0;
                }
                glyph += 1;
            }
            segments.push(Segment {
                glyphs: first..glyph,
                width,
                trailing,
                mandatory: bytes.get(end - 1) == Some(&b'\n'),
            });
        }
        segments
    }

    #[allow(clippy::too_many_arguments)]
    fn push_line(
        &self,
        lines: &mut Lines,
        fonts: &mut Fonts,
        styles: &[RunStyle],
        text: &[u8],
        range: Range<usize>,
        strut: RunStyle,
        justify_to: Option<f32>,
    ) {
        let (mut above, mut below) = vertical_extent(fonts, None, strut);
        let first_item = lines.items.len();
        let shaped = self.glyphs.get(range).unwrap_or_default();
        let is_space =
            |glyph: &ShapedGlyph| matches!(text.get(glyph.cluster as usize), Some(b' ' | b'\n'));
        let hanging = shaped
            .iter()
            .rev()
            .take_while(|glyph| is_space(glyph))
            .count();
        let shaped = shaped.get(..shaped.len() - hanging).unwrap_or_default();
        let level = |glyph: &ShapedGlyph| {
            self.spans
                .get(glyph.span as usize)
                .map_or(0, |span| span.level)
        };
        let order = visual_order(shaped, level);
        let natural: f32 = shaped.iter().map(|glyph| glyph.advance).sum();
        let spaces = shaped
            .iter()
            .filter(|glyph| text.get(glyph.cluster as usize) == Some(&b' '))
            .count();
        let expansion = match justify_to {
            Some(width) if spaces > 0 && width > natural => (width - natural) / spaces as f32,
            _ => 0.0,
        };
        let mut pen = 0.0;
        let visual: Vec<&ShapedGlyph> = order
            .iter()
            .filter_map(|&index| shaped.get(index))
            .collect();
        for group in visual.chunk_by(|a, b| a.span == b.span) {
            let Some((span, style)) = group.first().and_then(|first| {
                let span = *self.spans.get(first.span as usize)?;
                Some((span, *styles.get(span.run as usize)?))
            }) else {
                continue;
            };
            let (item_above, item_below) = vertical_extent(fonts, Some(span.font), style);
            above = f32::max(above, item_above);
            below = f32::max(below, item_below);
            let metrics = fonts.metrics(span.font);
            let scale = style.size / metrics.units_per_em;
            let start = pen;
            let glyph_start = lines.glyphs.len();
            for glyph in group {
                lines.glyphs.push(Glyph {
                    id: glyph.id,
                    x: pen - start + glyph.x_offset,
                    y: -glyph.y_offset,
                });
                pen += glyph.advance;
                if text.get(glyph.cluster as usize) == Some(&b' ') {
                    pen += expansion;
                }
            }
            lines.items.push(LineItem {
                tag: style.tag,
                font: span.font,
                size: style.size,
                x: start,
                width: pen - start,
                ascent: metrics.ascent * scale,
                descent: metrics.descent * scale,
                baseline_shift: style.baseline_shift,
                glyphs: glyph_start..lines.glyphs.len(),
            });
        }
        let top = lines.height;
        lines.lines.push(Line {
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
