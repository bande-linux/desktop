use std::borrow::Cow;
use std::ops::Range;

use parley::{Affinity, Alignment, AlignmentOptions, Cursor, Layout, Selection as Span};
use unbound_graphics::{Color, Paragraph, Point, Rect, TextStyle, TextSystem};

use super::selection::{Position, Selection};
use super::{CARET_WIDTH, Editor, Preedit, Unit};

const AVERAGE_ADVANCE: f32 = 0.55;
const KEPT_LAYOUTS: usize = 1024;

pub(super) struct Block {
    pub(super) text: String,
    pub(super) layout: Option<Paragraph>,
    height: Option<f32>,
    used: u64,
}

impl Block {
    pub(super) fn new(text: String) -> Self {
        Self {
            text,
            layout: None,
            height: None,
            used: 0,
        }
    }
}

pub(super) fn bounds(rect: parley::BoundingBox) -> Rect {
    Rect::new(
        rect.x0 as f32,
        rect.y0 as f32,
        (rect.x1 - rect.x0) as f32,
        (rect.y1 - rect.y0) as f32,
    )
}

impl Editor {
    pub(super) fn invalidate(&mut self) {
        for block in &mut self.blocks {
            block.layout = None;
            block.height = None;
        }
        self.laid_out = 0;
        self.tops_stale = true;
    }

    pub(super) fn invalidate_paragraph(&mut self, paragraph: usize) {
        if let Some(block) = self.blocks.get_mut(paragraph)
            && block.layout.take().is_some()
        {
            self.laid_out -= 1;
        }
    }

    pub(super) fn caret_paragraph(&self) -> Option<usize> {
        self.preedit
            .as_ref()
            .map(|_| self.selection.focus.paragraph)
    }

    pub(super) fn layout_text(&self, paragraph: usize) -> Cow<'_, str> {
        let text = &self.blocks[paragraph].text;
        match &self.preedit {
            Some(preedit) if self.caret_paragraph() == Some(paragraph) => {
                let at = self.selection.focus.offset;
                Cow::Owned(format!("{}{}{}", &text[..at], preedit.text, &text[at..]))
            }
            _ => Cow::Borrowed(text),
        }
    }

    pub(super) fn layout_index(&self, position: Position) -> usize {
        match &self.preedit {
            Some(preedit)
                if self.caret_paragraph() == Some(position.paragraph)
                    && position.offset >= self.selection.focus.offset
                    && position != self.selection.focus =>
            {
                position.offset + preedit.text.len()
            }
            _ => position.offset,
        }
    }

    fn build(&self, text: &mut TextSystem, paragraph: usize) -> Paragraph {
        let content = self.layout_text(paragraph);
        let (fonts, layouts) = text.contexts();
        let mut builder = layouts.ranged_builder(fonts, &content, self.scale, true);
        TextStyle::new(self.font_size, self.color).apply(&mut builder);
        let mut layout: Layout<Color> = builder.build(&content);
        layout.break_all_lines(self.width);
        layout.align(Alignment::Start, AlignmentOptions::default());
        Paragraph::from_layout(layout)
    }

    fn ensure(&mut self, text: &mut TextSystem, paragraph: usize) {
        let frame = self.frame;
        if self.blocks[paragraph].layout.is_none() {
            let built = self.build(text, paragraph);
            let height = built.height();
            if paragraph == 0 && self.blocks[0].text.is_empty() && self.preedit.is_none() {
                self.line_height = height;
            }
            let block = &mut self.blocks[paragraph];
            if block.height != Some(height) {
                self.tops_stale = true;
            }
            block.height = Some(height);
            block.layout = Some(built);
            self.laid_out += 1;
        }
        self.blocks[paragraph].used = frame;
    }

    pub(super) fn layout(&self, paragraph: usize) -> Option<&Layout<Color>> {
        self.blocks[paragraph]
            .layout
            .as_ref()
            .map(Paragraph::layout)
    }

    fn estimate(&self, block: &Block) -> f32 {
        if let Some(height) = block.height {
            return height;
        }
        let lines = match self.width {
            Some(width) if width > 0.0 => {
                let advance = block.text.len() as f32 * self.font_size * AVERAGE_ADVANCE;
                (advance / width).ceil().max(1.0)
            }
            _ => 1.0,
        };
        lines * self.line_height
    }

    fn refresh_tops(&mut self) {
        if !self.tops_stale && self.tops.len() == self.blocks.len() + 1 {
            return;
        }
        let mut tops = std::mem::take(&mut self.tops);
        tops.clear();
        tops.reserve(self.blocks.len() + 1);
        let mut y = 0.0;
        for block in &self.blocks {
            tops.push(y);
            y += self.estimate(block);
        }
        tops.push(y);
        self.tops = tops;
        self.tops_stale = false;
    }

    pub(super) fn top(&self, paragraph: usize) -> f32 {
        self.tops.get(paragraph).copied().unwrap_or_default()
    }

    pub(crate) fn first_line_height(&self) -> f32 {
        self.blocks[0].height.unwrap_or(self.line_height)
    }

    pub(crate) fn line_extent(&self) -> (f32, bool) {
        self.layout(0)
            .map_or((0.0, false), |layout| (layout.width(), layout.is_rtl()))
    }

    pub(crate) fn height(&mut self) -> f32 {
        self.refresh_tops();
        self.tops.last().copied().unwrap_or_default()
    }

    fn paragraph_at(&self, y: f32) -> usize {
        self.tops
            .partition_point(|top| *top <= y)
            .saturating_sub(1)
            .min(self.blocks.len() - 1)
    }

    pub(crate) fn lay_out(&mut self, text: &mut TextSystem, visible: Range<f32>) {
        self.frame += 1;
        self.refresh_tops();
        let mut paragraph = self.paragraph_at(visible.start);
        let mut y = self.top(paragraph);
        while paragraph < self.blocks.len() && (y < visible.end || paragraph == 0) {
            self.ensure(text, paragraph);
            y += self.estimate(&self.blocks[paragraph]);
            paragraph += 1;
        }
        let focus = self.selection.focus.paragraph;
        self.ensure(text, focus);
        self.refresh_tops();
        if self.laid_out > KEPT_LAYOUTS {
            let frame = self.frame;
            for block in &mut self.blocks {
                if block.used != frame && block.layout.take().is_some() {
                    self.laid_out -= 1;
                }
            }
        }
    }

    pub(super) fn cursor(
        &mut self,
        text: &mut TextSystem,
        position: Position,
        affinity: Affinity,
    ) -> Cursor {
        self.ensure(text, position.paragraph);
        let index = self.layout_index(position);
        match self.layout(position.paragraph) {
            Some(layout) => Cursor::from_byte_index(layout, index, affinity),
            None => Cursor::from_byte_index(&Layout::<Color>::new(), 0, affinity),
        }
    }

    pub(super) fn caret_in(&mut self, text: &mut TextSystem) -> Rect {
        let Selection {
            focus, affinity, ..
        } = self.selection;
        let mut index = self.layout_index(focus);
        if let Some(Preedit {
            cursor: Some((begin, _)),
            ..
        }) = &self.preedit
        {
            index = focus.offset + begin;
        }
        self.ensure(text, focus.paragraph);
        self.refresh_tops();
        let top = self.top(focus.paragraph);
        let Some(layout) = self.layout(focus.paragraph) else {
            return Rect::new(0.0, top, CARET_WIDTH, self.line_height);
        };
        let caret =
            bounds(Cursor::from_byte_index(layout, index, affinity).geometry(layout, CARET_WIDTH));
        Rect::new(caret.x, caret.y + top, caret.width, caret.height)
    }

    pub(crate) fn caret_rect(&mut self, text: &mut TextSystem) -> Rect {
        self.caret_in(text)
    }

    pub(crate) fn input_area(&self) -> Rect {
        let focus = self.selection.focus;
        let top = self.top(focus.paragraph);
        let Some(layout) = self.layout(focus.paragraph) else {
            return Rect::new(0.0, top, CARET_WIDTH, self.line_height);
        };
        let start =
            Cursor::from_byte_index(layout, self.layout_index(focus), self.selection.affinity);
        let mut area = bounds(start.geometry(layout, CARET_WIDTH));
        if let Some(preedit) = &self.preedit {
            let end = Cursor::from_byte_index(
                layout,
                focus.offset + preedit.text.len(),
                Affinity::Upstream,
            );
            Span::new(start, end).geometry_with(layout, |rect, _| {
                let rect = bounds(rect);
                if rect.y == area.y {
                    let right = (area.x + area.width).max(rect.x + rect.width);
                    area.x = area.x.min(rect.x);
                    area.width = right - area.x;
                }
            });
        }
        Rect::new(area.x, area.y + top, area.width, area.height)
    }

    pub(super) fn position_at(
        &mut self,
        text: &mut TextSystem,
        point: Point,
    ) -> (Position, Affinity) {
        self.refresh_tops();
        if point.y < 0.0 && self.multi_line {
            return (Position::default(), Affinity::Downstream);
        }
        let paragraph = self.paragraph_at(point.y);
        self.ensure(text, paragraph);
        let top = self.top(paragraph);
        let Some(layout) = self.layout(paragraph) else {
            return (Position::new(paragraph, 0), Affinity::Downstream);
        };
        let y = (point.y - top).clamp(0.0, (layout.height() - 1.0).max(0.0));
        let cursor = Cursor::from_point(layout, point.x, y);
        (Position::new(paragraph, cursor.index()), cursor.affinity())
    }

    pub(super) fn unit_at(
        &mut self,
        text: &mut TextSystem,
        point: Point,
        unit: Unit,
    ) -> (Position, Position) {
        let (at, _) = self.position_at(text, point);
        match unit {
            Unit::Character => (at, at),
            Unit::Word => {
                let top = self.top(at.paragraph);
                let Some(layout) = self.layout(at.paragraph) else {
                    return (at, at);
                };
                let y = (point.y - top).clamp(0.0, (layout.height() - 1.0).max(0.0));
                let word = Span::word_from_point(layout, point.x, y).text_range();
                (
                    Position::new(at.paragraph, word.start),
                    Position::new(at.paragraph, word.end),
                )
            }
            Unit::Paragraph => {
                let start = Position::new(at.paragraph, 0);
                let end = match at.paragraph + 1 < self.blocks.len() {
                    true => Position::new(at.paragraph + 1, 0),
                    false => Position::new(at.paragraph, self.blocks[at.paragraph].text.len()),
                };
                (start, end)
            }
        }
    }

    pub(super) fn visible(&self, visible: &Range<f32>) -> Range<usize> {
        let first = self.paragraph_at(visible.start);
        let last = self
            .tops
            .partition_point(|top| *top < visible.end)
            .clamp(first + 1, self.blocks.len());
        first..last
    }
}
