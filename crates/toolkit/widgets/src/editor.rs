use std::borrow::Cow;
use std::ops::Range;

use accesskit::{
    Action, ActionData, ActionRequest, Node, NodeId, Role, TextPosition, TextSelection,
};
use parley::{Affinity, Alignment, AlignmentOptions, Cursor, Layout, Selection as Span};
use unbound_graphics::{Color, Item, Paint, Paragraph, Point, Rect, Shape, TextStyle, TextSystem};

use super::{ClipboardRequest, Context, Event, Key, Menu, MenuItem, Modifiers, WidgetId};
use crate::L10N;

const CARET_WIDTH: f32 = 1.0;
const AVERAGE_ADVANCE: f32 = 0.55;
const LINE_HEIGHT_RATIO: f32 = 1.21;
const KEPT_LAYOUTS: usize = 1024;
const UNDERLINE: f32 = 1.0;
const CLAUSE_UNDERLINE: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) struct Position {
    pub(crate) paragraph: usize,
    pub(crate) offset: usize,
}

impl Position {
    const fn new(paragraph: usize, offset: usize) -> Self {
        Self { paragraph, offset }
    }

    fn after(self, text: &str) -> Self {
        match text.rfind('\n') {
            Some(last) => Self::new(
                self.paragraph + text.matches('\n').count(),
                text.len() - last - 1,
            ),
            None => Self::new(self.paragraph, self.offset + text.len()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Selection {
    anchor: Position,
    focus: Position,
    affinity: Affinity,
}

impl Selection {
    fn caret(at: Position, affinity: Affinity) -> Self {
        Self {
            anchor: at,
            focus: at,
            affinity,
        }
    }

    fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }

    fn start(&self) -> Position {
        self.anchor.min(self.focus)
    }

    fn end(&self) -> Position {
        self.anchor.max(self.focus)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Typing,
    Deleting,
    Replacing,
}

#[derive(Debug, Clone)]
struct Edit {
    start: Position,
    removed: String,
    inserted: String,
    before: Selection,
    kind: EditKind,
}

#[derive(Default)]
struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    open: bool,
}

impl History {
    fn record(&mut self, edit: Edit) {
        self.redo.clear();
        if self.open
            && let Some(last) = self.undo.last_mut()
            && last.kind == edit.kind
            && coalesce(last, &edit)
        {
            return;
        }
        self.undo.push(edit);
        self.open = true;
    }
}

fn coalesce(last: &mut Edit, edit: &Edit) -> bool {
    match edit.kind {
        EditKind::Typing => {
            let contiguous =
                edit.removed.is_empty() && last.start.after(&last.inserted) == edit.start;
            let starts_word = !edit.inserted.starts_with(char::is_whitespace)
                && last.inserted.ends_with(char::is_whitespace);
            if !contiguous || starts_word {
                return false;
            }
            last.inserted.push_str(&edit.inserted);
            true
        }
        EditKind::Deleting if !last.inserted.is_empty() || !edit.inserted.is_empty() => false,
        EditKind::Deleting if edit.start.after(&edit.removed) == last.start => {
            last.removed.insert_str(0, &edit.removed);
            last.start = edit.start;
            true
        }
        EditKind::Deleting if edit.start == last.start => {
            last.removed.push_str(&edit.removed);
            true
        }
        _ => false,
    }
}

struct Block {
    text: String,
    layout: Option<Paragraph>,
    height: Option<f32>,
    used: u64,
}

impl Block {
    fn new(text: String) -> Self {
        Self {
            text,
            layout: None,
            height: None,
            used: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Preedit {
    text: String,
    cursor: Option<(usize, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Character,
    Word,
    Paragraph,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    unit: Unit,
    anchor: (Position, Position),
}

pub(crate) struct TextRun {
    paragraph: usize,
    range: Range<usize>,
    rect: Rect,
    newline: bool,
}

pub(crate) struct Editor {
    blocks: Vec<Block>,
    tops: Vec<f32>,
    tops_stale: bool,
    selection: Selection,
    goal_x: Option<f32>,
    preedit: Option<Preedit>,
    history: History,
    font_size: f32,
    multi_line: bool,
    width: Option<f32>,
    color: Color,
    line_height: f32,
    drag: Option<Drag>,
    drag_point: Option<Point>,
    reveal: bool,
    read_only: bool,
    page: f32,
    frame: u64,
    laid_out: usize,
    revision: u64,
}

fn normalized(text: &str) -> Cow<'_, str> {
    if text.contains('\r') {
        Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

fn bounds(rect: parley::BoundingBox) -> Rect {
    Rect::new(
        rect.x0 as f32,
        rect.y0 as f32,
        (rect.x1 - rect.x0) as f32,
        (rect.y1 - rect.y0) as f32,
    )
}

impl Editor {
    pub(crate) fn new(text: &str, font_size: f32, multi_line: bool) -> Self {
        let mut editor = Self {
            blocks: vec![Block::new(String::new())],
            tops: Vec::new(),
            tops_stale: true,
            selection: Selection::caret(Position::default(), Affinity::Downstream),
            goal_x: None,
            preedit: None,
            history: History::default(),
            font_size,
            multi_line,
            width: None,
            color: Color::hex(0),
            line_height: (font_size * LINE_HEIGHT_RATIO).ceil(),
            drag: None,
            drag_point: None,
            reveal: false,
            read_only: false,
            page: 0.0,
            frame: 0,
            laid_out: 0,
            revision: 0,
        };
        editor.set_text(text);
        editor
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn font_size(&self) -> f32 {
        self.font_size
    }

    pub(crate) fn set_text(&mut self, text: &str) {
        let text = self.accepted(text);
        self.blocks = text
            .split('\n')
            .map(|line| Block::new(line.to_owned()))
            .collect();
        self.laid_out = 0;
        self.tops_stale = true;
        self.preedit = None;
        self.history = History::default();
        let end = self.end();
        self.selection = Selection::caret(end, Affinity::Upstream);
        self.revision += 1;
    }

    fn accepted<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let text = normalized(text);
        if self.multi_line || !text.contains('\n') {
            return text;
        }
        Cow::Owned(text.replace('\n', " "))
    }

    pub(crate) fn text(&self) -> String {
        let mut text =
            String::with_capacity(self.blocks.iter().map(|block| block.text.len() + 1).sum());
        for (index, block) in self.blocks.iter().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            text.push_str(&block.text);
        }
        text
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.blocks.len() == 1 && self.blocks[0].text.is_empty() && self.preedit.is_none()
    }

    fn end(&self) -> Position {
        let last = self.blocks.len() - 1;
        Position::new(last, self.blocks[last].text.len())
    }

    fn byte_offset(&self, position: Position) -> usize {
        self.blocks[..position.paragraph]
            .iter()
            .map(|block| block.text.len() + 1)
            .sum::<usize>()
            + position.offset
    }

    fn position_of(&self, mut offset: usize) -> Position {
        for (paragraph, block) in self.blocks.iter().enumerate() {
            if offset <= block.text.len() {
                let mut offset = offset;
                while !block.text.is_char_boundary(offset) {
                    offset -= 1;
                }
                return Position::new(paragraph, offset);
            }
            offset -= block.text.len() + 1;
        }
        self.end()
    }

    pub(crate) fn selection(&self) -> Range<usize> {
        self.byte_offset(self.selection.start())..self.byte_offset(self.selection.end())
    }

    pub(crate) fn select(&mut self, range: Range<usize>) {
        self.selection = Selection {
            anchor: self.position_of(range.start),
            focus: self.position_of(range.end),
            affinity: Affinity::Downstream,
        };
        self.goal_x = None;
        self.history.open = false;
    }

    pub(crate) fn selected_text(&self) -> String {
        self.slice(self.selection.start(), self.selection.end())
    }

    pub(crate) fn has_selection(&self) -> bool {
        !self.selection.is_collapsed()
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.history.undo.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.history.redo.is_empty()
    }

    fn slice(&self, start: Position, end: Position) -> String {
        if start.paragraph == end.paragraph {
            return self.blocks[start.paragraph].text[start.offset..end.offset].to_owned();
        }
        let mut text = self.blocks[start.paragraph].text[start.offset..].to_owned();
        for block in &self.blocks[start.paragraph + 1..end.paragraph] {
            text.push('\n');
            text.push_str(&block.text);
        }
        text.push('\n');
        text.push_str(&self.blocks[end.paragraph].text[..end.offset]);
        text
    }

    fn splice(&mut self, start: Position, end: Position, text: &str) -> Position {
        let tail = self.blocks[end.paragraph].text[end.offset..].to_owned();
        let head = &self.blocks[start.paragraph].text[..start.offset];
        let mut lines = text.split('\n');
        let mut texts = vec![format!("{head}{}", lines.next().unwrap_or_default())];
        texts.extend(lines.map(str::to_owned));
        let last = texts.len() - 1;
        let end_position = Position::new(start.paragraph + last, texts[last].len());
        texts[last].push_str(&tail);
        let removed = self.blocks.splice(
            start.paragraph..=end.paragraph,
            texts.into_iter().map(Block::new),
        );
        let dropped = removed.filter(|block| block.layout.is_some()).count();
        self.laid_out -= dropped;
        self.tops_stale = true;
        self.revision += 1;
        end_position
    }

    fn replace(&mut self, start: Position, end: Position, text: &str, kind: EditKind) {
        let before = self.selection;
        let removed = self.slice(start, end);
        if removed.is_empty() && text.is_empty() {
            return;
        }
        let caret = self.splice(start, end, text);
        self.selection = Selection::caret(caret, Affinity::Downstream);
        self.goal_x = None;
        self.history.record(Edit {
            start,
            removed,
            inserted: text.to_owned(),
            before,
            kind,
        });
    }

    fn replace_selection(&mut self, text: &str, kind: EditKind) {
        let kind = match self.selection.is_collapsed() {
            true => kind,
            false => EditKind::Replacing,
        };
        let (start, end) = (self.selection.start(), self.selection.end());
        if kind == EditKind::Replacing {
            self.history.open = false;
        }
        self.replace(start, end, text, kind);
        if kind == EditKind::Replacing && text.chars().count() == 1 {
            if let Some(last) = self.history.undo.last_mut() {
                last.kind = EditKind::Typing;
            }
        } else if kind == EditKind::Replacing {
            self.history.open = false;
        }
    }

    pub(crate) fn insert(&mut self, text: &str) {
        let text = self.accepted(text).into_owned();
        let kind = match text.chars().count() {
            1 => EditKind::Typing,
            _ => EditKind::Replacing,
        };
        if kind == EditKind::Replacing {
            self.history.open = false;
        }
        self.replace_selection(&text, kind);
        if kind == EditKind::Replacing {
            self.history.open = false;
        }
    }

    pub(crate) fn clear(&mut self) {
        self.preedit = None;
        self.history.open = false;
        let end = self.end();
        self.replace(Position::default(), end, "", EditKind::Replacing);
        self.history.open = false;
    }

    pub(crate) fn undo(&mut self) -> bool {
        if self.preedit.is_some() {
            return false;
        }
        let Some(edit) = self.history.undo.pop() else {
            return false;
        };
        let end = edit.start.after(&edit.inserted);
        self.splice(edit.start, end, &edit.removed);
        self.selection = edit.before;
        self.goal_x = None;
        self.history.redo.push(edit);
        self.history.open = false;
        true
    }

    pub(crate) fn redo(&mut self) -> bool {
        if self.preedit.is_some() {
            return false;
        }
        let Some(edit) = self.history.redo.pop() else {
            return false;
        };
        let end = edit.start.after(&edit.removed);
        let caret = self.splice(edit.start, end, &edit.inserted);
        self.selection = Selection::caret(caret, Affinity::Downstream);
        self.goal_x = None;
        self.history.undo.push(edit);
        self.history.open = false;
        true
    }

    pub(crate) fn set_width(&mut self, width: Option<f32>) {
        if self.width == width {
            return;
        }
        self.width = width;
        self.invalidate();
    }

    pub(crate) fn set_color(&mut self, color: Color) {
        if self.color == color {
            return;
        }
        self.color = color;
        self.invalidate();
    }

    pub(crate) fn set_page(&mut self, page: f32) {
        self.page = page;
    }

    fn invalidate(&mut self) {
        for block in &mut self.blocks {
            block.layout = None;
            block.height = None;
        }
        self.laid_out = 0;
        self.tops_stale = true;
    }

    fn invalidate_paragraph(&mut self, paragraph: usize) {
        if let Some(block) = self.blocks.get_mut(paragraph)
            && block.layout.take().is_some()
        {
            self.laid_out -= 1;
        }
    }

    fn caret_paragraph(&self) -> Option<usize> {
        self.preedit
            .as_ref()
            .map(|_| self.selection.focus.paragraph)
    }

    fn layout_text(&self, paragraph: usize) -> Cow<'_, str> {
        let text = &self.blocks[paragraph].text;
        match &self.preedit {
            Some(preedit) if self.caret_paragraph() == Some(paragraph) => {
                let at = self.selection.focus.offset;
                Cow::Owned(format!("{}{}{}", &text[..at], preedit.text, &text[at..]))
            }
            _ => Cow::Borrowed(text),
        }
    }

    fn layout_index(&self, position: Position) -> usize {
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
        let mut builder = layouts.ranged_builder(fonts, &content, 1.0, true);
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

    fn layout(&self, paragraph: usize) -> Option<&Layout<Color>> {
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

    fn top(&self, paragraph: usize) -> f32 {
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

    fn cursor(&mut self, text: &mut TextSystem, position: Position, affinity: Affinity) -> Cursor {
        self.ensure(text, position.paragraph);
        let index = self.layout_index(position);
        match self.layout(position.paragraph) {
            Some(layout) => Cursor::from_byte_index(layout, index, affinity),
            None => Cursor::from_byte_index(&Layout::<Color>::new(), 0, affinity),
        }
    }

    fn caret_in(&mut self, text: &mut TextSystem) -> Rect {
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

    fn position_at(&mut self, text: &mut TextSystem, point: Point) -> (Position, Affinity) {
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

    fn unit_at(&mut self, text: &mut TextSystem, point: Point, unit: Unit) -> (Position, Position) {
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

    fn press(&mut self, cx: &mut Context, point: Point, count: u8, extend: bool) {
        if self.preedit.is_some() {
            self.finish_composition();
            cx.input_method_reset = true;
        }
        self.history.open = false;
        self.goal_x = None;
        let unit = match count {
            0 | 1 => Unit::Character,
            2 => Unit::Word,
            _ => Unit::Paragraph,
        };
        if extend && unit == Unit::Character {
            let (focus, affinity) = self.position_at(&mut cx.text, point);
            self.selection.focus = focus;
            self.selection.affinity = affinity;
            let anchor = self.selection.anchor;
            self.drag = Some(Drag {
                unit,
                anchor: (anchor, anchor),
            });
            return;
        }
        let (start, end) = self.unit_at(&mut cx.text, point, unit);
        let (_, affinity) = self.position_at(&mut cx.text, point);
        self.selection = Selection {
            anchor: start,
            focus: end,
            affinity,
        };
        self.drag = Some(Drag {
            unit,
            anchor: (start, end),
        });
    }

    fn drag_to(&mut self, text: &mut TextSystem, point: Point) -> bool {
        let Some(Drag { unit, anchor }) = self.drag else {
            return false;
        };
        let (start, end) = self.unit_at(text, point, unit);
        let (_, affinity) = self.position_at(text, point);
        let before = self.selection;
        self.selection = match start < anchor.0 {
            true => Selection {
                anchor: anchor.1,
                focus: start,
                affinity,
            },
            false => Selection {
                anchor: anchor.0,
                focus: end.max(anchor.1),
                affinity,
            },
        };
        self.selection != before
    }

    pub(crate) fn handle(&mut self, cx: &mut Context, event: &Event, origin: Point) -> bool {
        let local = |point: Point| Point::new(point.x - origin.x, point.y - origin.y);
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => {
                self.press(
                    cx,
                    local(pointer.position),
                    pointer.count,
                    pointer.modifiers.shift,
                );
                self.drag_point = Some(pointer.position);
            }
            Event::PointerDown(pointer) => {
                self.place_caret_for_menu(&mut cx.text, local(pointer.position));
            }
            Event::PointerMove(pointer) if self.drag.is_some() => {
                self.drag_to(&mut cx.text, local(pointer.position));
                self.drag_point = Some(pointer.position);
            }
            Event::PointerUp(pointer) if pointer.is_primary() => {
                self.drag = None;
                self.drag_point = None;
            }
            Event::Key { key, modifiers } => {
                if !self.key(cx, *key, *modifiers) {
                    return false;
                }
            }
            Event::Commit(text) if !self.read_only => self.commit(text),
            Event::Preedit { text, cursor } if !self.read_only => {
                if !self.set_preedit(text, *cursor) {
                    return false;
                }
            }
            _ => return false,
        }
        self.reveal = true;
        true
    }

    pub(crate) fn autoscroll(
        &mut self,
        text: &mut TextSystem,
        origin: Point,
        content: Rect,
    ) -> bool {
        let Some(point) = self.drag_point else {
            return false;
        };
        let inside = point.x >= content.x
            && point.x <= content.x + content.width
            && point.y >= content.y
            && point.y <= content.y + content.height;
        if inside {
            return false;
        }
        self.drag_to(text, Point::new(point.x - origin.x, point.y - origin.y));
        self.reveal = true;
        true
    }

    pub(crate) fn take_reveal(&mut self) -> bool {
        std::mem::take(&mut self.reveal)
    }

    pub(crate) fn perform(
        &mut self,
        owner: WidgetId,
        runs: &[TextRun],
        request: &ActionRequest,
    ) -> bool {
        match (request.action, &request.data) {
            (Action::SetValue, Some(ActionData::Value(value))) if !self.read_only => {
                self.set_text(value);
            }
            (Action::ReplaceSelectedText, Some(ActionData::Value(value))) if !self.read_only => {
                self.insert(value);
            }
            (Action::SetTextSelection, Some(ActionData::SetTextSelection(selection))) => {
                if !self.set_text_selection(runs, selection, owner) {
                    return false;
                }
            }
            _ => return false,
        }
        self.reveal = true;
        true
    }

    pub(crate) fn describe(&self, node: &mut Node, owner: WidgetId, runs: &[TextRun]) {
        if let Some(selection) = self.text_selection(owner, runs) {
            node.set_text_selection(selection);
        }
        match self.read_only {
            true => node.set_read_only(),
            false => {
                node.add_action(Action::SetValue);
                node.add_action(Action::ReplaceSelectedText);
            }
        }
        node.add_action(Action::SetTextSelection);
    }

    pub(crate) fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    pub(crate) fn select_all(&mut self) {
        self.selection = Selection {
            anchor: Position::default(),
            focus: self.end(),
            affinity: Affinity::Upstream,
        };
        self.history.open = false;
    }

    fn go(&mut self, to: (Position, Affinity), extend: bool) {
        match extend {
            true => {
                self.selection.focus = to.0;
                self.selection.affinity = to.1;
            }
            false => self.selection = Selection::caret(to.0, to.1),
        }
        self.goal_x = None;
        self.history.open = false;
    }

    fn horizontal(
        &mut self,
        text: &mut TextSystem,
        right: bool,
        word: bool,
    ) -> (Position, Affinity) {
        let Selection {
            focus, affinity, ..
        } = self.selection;
        let cursor = self.cursor(text, focus, affinity);
        let Some(layout) = self.layout(focus.paragraph) else {
            return (focus, affinity);
        };
        let next = match (right, word) {
            (true, false) => cursor.next_visual(layout),
            (false, false) => cursor.previous_visual(layout),
            (true, true) => cursor.next_visual_word(layout),
            (false, true) => cursor.previous_visual_word(layout),
        };
        if next != cursor {
            return (
                Position::new(focus.paragraph, next.index()),
                next.affinity(),
            );
        }
        let forward = right != layout.is_rtl();
        match forward {
            true if focus.paragraph + 1 < self.blocks.len() => {
                (Position::new(focus.paragraph + 1, 0), Affinity::Downstream)
            }
            false if focus.paragraph > 0 => {
                let previous = focus.paragraph - 1;
                (
                    Position::new(previous, self.blocks[previous].text.len()),
                    Affinity::Upstream,
                )
            }
            _ => (focus, affinity),
        }
    }

    fn line_edge(&mut self, text: &mut TextSystem, end: bool) -> (Position, Affinity) {
        let Selection {
            focus, affinity, ..
        } = self.selection;
        let cursor = self.cursor(text, focus, affinity);
        let Some(layout) = self.layout(focus.paragraph) else {
            return (focus, affinity);
        };
        let span = Span::from(cursor);
        let moved = match end {
            true => span.line_end(layout, false),
            false => span.line_start(layout, false),
        }
        .focus();
        (
            Position::new(focus.paragraph, moved.index()),
            moved.affinity(),
        )
    }

    fn vertical(&mut self, text: &mut TextSystem, distance: f32) -> (Position, Affinity) {
        let caret = self.caret_in(text);
        let x = *self.goal_x.get_or_insert(caret.x);
        let y = match distance < 0.0 {
            true => caret.y + distance,
            false => caret.y + caret.height + distance,
        };
        let height = self.height();
        if y < 0.0 {
            return (Position::default(), Affinity::Downstream);
        }
        if y >= height {
            return (self.end(), Affinity::Upstream);
        }
        self.position_at(text, Point::new(x, y))
    }

    fn paragraph_edge(&self, forward: bool) -> (Position, Affinity) {
        let focus = self.selection.focus;
        let len = self.blocks[focus.paragraph].text.len();
        match forward {
            true if focus.offset < len => (Position::new(focus.paragraph, len), Affinity::Upstream),
            true if focus.paragraph + 1 < self.blocks.len() => {
                let next = focus.paragraph + 1;
                (
                    Position::new(next, self.blocks[next].text.len()),
                    Affinity::Upstream,
                )
            }
            false if focus.offset > 0 => (Position::new(focus.paragraph, 0), Affinity::Downstream),
            false if focus.paragraph > 0 => {
                (Position::new(focus.paragraph - 1, 0), Affinity::Downstream)
            }
            _ => (focus, self.selection.affinity),
        }
    }

    fn move_vertically(&mut self, text: &mut TextSystem, distance: f32, extend: bool) {
        if !extend && !self.selection.is_collapsed() {
            let edge = match distance < 0.0 {
                true => self.selection.start(),
                false => self.selection.end(),
            };
            self.selection = Selection::caret(edge, self.selection.affinity);
        }
        let goal = self.goal_x;
        let to = self.vertical(text, distance);
        let goal = goal.or(self.goal_x);
        self.go(to, extend);
        self.goal_x = goal;
    }

    fn deletion(
        &mut self,
        text: &mut TextSystem,
        forward: bool,
        word: bool,
    ) -> Option<(Position, Position)> {
        if !self.selection.is_collapsed() {
            return Some((self.selection.start(), self.selection.end()));
        }
        let focus = self.selection.focus;
        let cursor = self.cursor(text, focus, self.selection.affinity);
        let layout = self.layout(focus.paragraph)?;
        let block = &self.blocks[focus.paragraph].text;
        if forward && focus.offset == block.len() {
            return (focus.paragraph + 1 < self.blocks.len())
                .then(|| (focus, Position::new(focus.paragraph + 1, 0)));
        }
        if !forward && focus.offset == 0 {
            return (focus.paragraph > 0).then(|| {
                let previous = focus.paragraph - 1;
                (
                    Position::new(previous, self.blocks[previous].text.len()),
                    focus,
                )
            });
        }
        let other = match (forward, word) {
            (true, true) => cursor.next_logical_word(layout).index(),
            (false, true) => cursor.previous_logical_word(layout).index(),
            (true, false) => cursor.logical_clusters(layout)[1]
                .as_ref()
                .map_or(block.len(), |cluster| cluster.text_range().end),
            (false, false) => match &cursor.logical_clusters(layout)[0] {
                Some(cluster) if cluster.is_emoji() => cluster.text_range().start,
                _ => block[..focus.offset]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(index, _)| index),
            },
        };
        let other = Position::new(focus.paragraph, other);
        Some((focus.min(other), focus.max(other)))
    }

    fn delete(&mut self, text: &mut TextSystem, forward: bool, word: bool) {
        if let Some((start, end)) = self.deletion(text, forward, word) {
            let kind = match self.selection.is_collapsed() && !word {
                true => EditKind::Deleting,
                false => EditKind::Replacing,
            };
            if kind == EditKind::Replacing {
                self.history.open = false;
            }
            self.replace(start, end, "", kind);
            if kind == EditKind::Replacing {
                self.history.open = false;
            }
        }
    }

    pub(crate) fn key(&mut self, cx: &mut Context, key: Key, modifiers: Modifiers) -> bool {
        if self.preedit.is_some() {
            return false;
        }
        let Modifiers {
            shift,
            alt,
            logo,
            control,
        } = modifiers;
        if control {
            return false;
        }
        let edits = match key {
            Key::Character('z' | 'Z' | 'x' | 'v') if logo => true,
            Key::Character(_) | Key::Space => !logo,
            Key::Backspace | Key::Delete | Key::Enter => true,
            _ => false,
        };
        if edits && self.read_only {
            return false;
        }
        let text = &mut cx.text;
        match key {
            Key::Character('z' | 'Z') if logo && shift => {
                self.redo();
            }
            Key::Character('z') if logo => {
                self.undo();
            }
            Key::Character('a') if logo => self.select_all(),
            Key::Character('c' | 'x') if logo => {
                if self.selection.is_collapsed() {
                    return true;
                }
                cx.clipboard = Some(ClipboardRequest::Copy(self.selected_text()));
                if key == Key::Character('x') {
                    self.history.open = false;
                    let (start, end) = (self.selection.start(), self.selection.end());
                    self.replace(start, end, "", EditKind::Replacing);
                    self.history.open = false;
                }
            }
            Key::Character('v') if logo => cx.clipboard = Some(ClipboardRequest::Paste),
            Key::Left | Key::Right if !shift && !alt && !logo && !self.selection.is_collapsed() => {
                let edge = match key == Key::Right {
                    true => self.selection.end(),
                    false => self.selection.start(),
                };
                self.go((edge, self.selection.affinity), false);
            }
            Key::Left | Key::Right if logo => {
                let to = self.line_edge(text, key == Key::Right);
                self.go(to, shift);
            }
            Key::Home | Key::End => {
                let to = self.line_edge(text, key == Key::End);
                self.go(to, shift);
            }
            Key::Left | Key::Right => {
                let to = self.horizontal(text, key == Key::Right, alt);
                self.go(to, shift);
            }
            Key::Up | Key::Down if logo => {
                let to = match key == Key::Down {
                    true => (self.end(), Affinity::Upstream),
                    false => (Position::default(), Affinity::Downstream),
                };
                self.go(to, shift);
            }
            Key::Up | Key::Down if alt => {
                let to = self.paragraph_edge(key == Key::Down);
                self.go(to, shift);
            }
            Key::Up => self.move_vertically(text, -1.0, shift),
            Key::Down => self.move_vertically(text, 1.0, shift),
            Key::PageUp if self.multi_line => {
                self.move_vertically(text, -self.page.max(self.line_height), shift);
            }
            Key::PageDown if self.multi_line => {
                self.move_vertically(text, self.page.max(self.line_height), shift);
            }
            Key::Backspace if logo => {
                if self.selection.is_collapsed() {
                    let to = self.line_edge(text, false);
                    self.go(to, true);
                }
                self.delete(text, false, false);
            }
            Key::Backspace => self.delete(text, false, alt),
            Key::Delete => self.delete(text, true, alt),
            Key::Enter if self.multi_line && !logo => self.insert("\n"),
            Key::Character(character) if !logo => {
                self.replace_selection(character.encode_utf8(&mut [0; 4]), EditKind::Typing);
            }
            Key::Space if !logo => self.replace_selection(" ", EditKind::Typing),
            _ => return false,
        }
        true
    }

    pub(crate) fn commit(&mut self, text: &str) {
        if let Some(preedit) = self.preedit.take() {
            self.invalidate_paragraph(self.selection.focus.paragraph);
            if preedit.text == text {
                self.revision += 1;
            }
        }
        let text = self.accepted(text).into_owned();
        let kind = match text.chars().count() {
            1 => EditKind::Typing,
            _ => EditKind::Replacing,
        };
        if kind == EditKind::Replacing {
            self.history.open = false;
        }
        self.replace_selection(&text, kind);
        if kind == EditKind::Replacing {
            self.history.open = false;
        }
    }

    pub(crate) fn set_preedit(&mut self, text: &str, cursor: Option<(usize, usize)>) -> bool {
        let preedit = (!text.is_empty()).then(|| Preedit {
            text: self.accepted(text).replace('\n', " "),
            cursor,
        });
        if preedit == self.preedit {
            return false;
        }
        if preedit.is_some() && self.preedit.is_none() && !self.selection.is_collapsed() {
            self.history.open = false;
            let (start, end) = (self.selection.start(), self.selection.end());
            self.replace(start, end, "", EditKind::Replacing);
        }
        self.invalidate_paragraph(self.selection.focus.paragraph);
        self.preedit = preedit;
        self.invalidate_paragraph(self.selection.focus.paragraph);
        self.tops_stale = true;
        true
    }

    fn finish_composition(&mut self) {
        if let Some(preedit) = self.preedit.take() {
            self.invalidate_paragraph(self.selection.focus.paragraph);
            self.replace_selection(&preedit.text, EditKind::Replacing);
            self.history.open = false;
        }
    }

    fn set_text_selection(
        &mut self,
        runs: &[TextRun],
        selection: &TextSelection,
        owner: WidgetId,
    ) -> bool {
        let locate = |position: &TextPosition| {
            let index = WidgetId::owner_of_part(position.node)
                .filter(|(id, _)| *id == owner)
                .map(|(_, part)| part as usize)?;
            let run = runs.get(index)?;
            let text = self.layout_text(run.paragraph);
            let offset = text[run.range.clone()]
                .char_indices()
                .nth(position.character_index)
                .map_or(run.range.end, |(at, _)| run.range.start + at);
            Some(Position::new(run.paragraph, offset))
        };
        let (Some(anchor), Some(focus)) = (locate(&selection.anchor), locate(&selection.focus))
        else {
            return false;
        };
        self.selection = Selection {
            anchor,
            focus,
            affinity: Affinity::Downstream,
        };
        self.history.open = false;
        true
    }

    pub(crate) fn menu(&self) -> Menu {
        let item = |key: &str, shortcut: &str, enabled: bool| {
            MenuItem::new(&L10N.tr(key), key)
                .shortcut(shortcut)
                .enabled(enabled)
        };
        let selected = self.has_selection();
        if self.read_only {
            return Menu::new(vec![
                item("copy", "Super+C", selected),
                MenuItem::separator(),
                item("select-all", "Super+A", !self.is_empty()),
            ]);
        }
        Menu::new(vec![
            item("undo", "Super+Z", self.can_undo()),
            item("redo", "Shift+Super+Z", self.can_redo()),
            MenuItem::separator(),
            item("cut", "Super+X", selected),
            item("copy", "Super+C", selected),
            item("paste", "Super+V", true),
            MenuItem::separator(),
            item("select-all", "Super+A", !self.is_empty()),
            MenuItem::separator(),
            item("emoji-and-symbols", "Ctrl+Super+Space", true),
        ])
    }

    pub(crate) fn menu_action(&mut self, cx: &mut Context, action: &str) -> bool {
        let shortcut = |character: char, shift: bool| {
            (
                Key::Character(character),
                Modifiers {
                    shift,
                    logo: true,
                    ..Modifiers::default()
                },
            )
        };
        let (key, modifiers) = match action {
            "undo" => shortcut('z', false),
            "redo" => shortcut('z', true),
            "cut" => shortcut('x', false),
            "copy" => shortcut('c', false),
            "paste" => shortcut('v', false),
            "select-all" => shortcut('a', false),
            "emoji-and-symbols" => {
                cx.show_emoji_picker();
                return true;
            }
            _ => return false,
        };
        self.key(cx, key, modifiers)
    }

    fn place_caret_for_menu(&mut self, text: &mut TextSystem, point: Point) {
        let (at, affinity) = self.position_at(text, point);
        if self.selection.is_collapsed() || at < self.selection.start() || at > self.selection.end()
        {
            self.go((at, affinity), false);
        }
    }

    fn visible(&self, visible: &Range<f32>) -> Range<usize> {
        let first = self.paragraph_at(visible.start);
        let last = self
            .tops
            .partition_point(|top| *top < visible.end)
            .clamp(first + 1, self.blocks.len());
        first..last
    }

    pub(crate) fn paint(
        &self,
        cx: &Context,
        origin: Point,
        visible: Range<f32>,
        focused: bool,
        items: &mut Vec<Item>,
    ) {
        let (start, end) = (self.selection.start(), self.selection.end());
        let selection_color = match (focused, self.multi_line) {
            (true, _) => cx.palette.accent.with_alpha(80),
            (false, true) => cx.palette.ink2.with_alpha(56),
            (false, false) => Color::TRANSPARENT,
        };
        for paragraph in self.visible(&visible) {
            let Some(text) = &self.blocks[paragraph].layout else {
                continue;
            };
            let layout = text.layout();
            let top = origin.y + self.top(paragraph);
            let at =
                |rect: Rect| Rect::new(origin.x + rect.x, top + rect.y, rect.width, rect.height);
            if start.paragraph <= paragraph && paragraph <= end.paragraph && start != end {
                let from = match paragraph == start.paragraph {
                    true => self.layout_index(start),
                    false => 0,
                };
                let to = match paragraph == end.paragraph {
                    true => self.layout_index(end),
                    false => layout.text_len(),
                };
                let span = Span::new(
                    Cursor::from_byte_index(layout, from, Affinity::Downstream),
                    Cursor::from_byte_index(layout, to, Affinity::Upstream),
                );
                span.geometry_with(layout, |rect, _| {
                    items.push(Item::Fill {
                        shape: Shape::rect(at(bounds(rect))),
                        paint: Paint::Solid(selection_color),
                    });
                });
                if paragraph < end.paragraph {
                    let edge = bounds(
                        Cursor::from_byte_index(layout, layout.text_len(), Affinity::Upstream)
                            .geometry(layout, 0.0),
                    );
                    let right = self.width.unwrap_or(edge.x + self.font_size * 0.3);
                    items.push(Item::Fill {
                        shape: Shape::rect(at(Rect::new(
                            edge.x,
                            edge.y,
                            (right - edge.x).max(self.font_size * 0.3),
                            edge.height,
                        ))),
                        paint: Paint::Solid(selection_color),
                    });
                }
            }
            items.push(Item::Text {
                origin: Point::new(origin.x, top),
                paragraph: text.clone(),
            });
            if let Some(preedit) = &self.preedit
                && self.caret_paragraph() == Some(paragraph)
            {
                let at_offset = self.selection.focus.offset;
                let underline = |from: usize, to: usize, thickness: f32, items: &mut Vec<Item>| {
                    let span = Span::new(
                        Cursor::from_byte_index(layout, at_offset + from, Affinity::Downstream),
                        Cursor::from_byte_index(layout, at_offset + to, Affinity::Upstream),
                    );
                    span.geometry_with(layout, |rect, _| {
                        let rect = at(bounds(rect));
                        items.push(Item::Fill {
                            shape: Shape::rect(Rect::new(
                                rect.x,
                                rect.y + rect.height - thickness - 1.0,
                                rect.width,
                                thickness,
                            )),
                            paint: cx.palette.ink.into(),
                        });
                    });
                };
                underline(0, preedit.text.len(), UNDERLINE, items);
                if let Some((begin, end)) = preedit.cursor
                    && begin != end
                {
                    underline(begin.min(end), begin.max(end), CLAUSE_UNDERLINE, items);
                }
            }
        }
        let caret_hidden = matches!(&self.preedit, Some(Preedit { cursor: None, .. }))
            || matches!(&self.preedit, Some(Preedit { cursor: Some((begin, end)), .. }) if begin != end);
        let focus = self.selection.focus.paragraph;
        if focused
            && !self.read_only
            && self.selection.is_collapsed()
            && !caret_hidden
            && let Some(text) = &self.blocks[focus].layout
        {
            let layout = text.layout();
            let mut index = self.layout_index(self.selection.focus);
            if let Some(Preedit {
                cursor: Some((begin, _)),
                ..
            }) = &self.preedit
            {
                index = self.selection.focus.offset + begin;
            }
            let caret = bounds(
                Cursor::from_byte_index(layout, index, self.selection.affinity)
                    .geometry(layout, CARET_WIDTH),
            );
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    origin.x + caret.x,
                    origin.y + self.top(focus) + caret.y,
                    CARET_WIDTH,
                    caret.height,
                )),
                paint: cx.palette.ink.into(),
            });
        }
    }

    pub(crate) fn runs(&self, visible: Range<f32>) -> Vec<TextRun> {
        let mut runs = Vec::new();
        for paragraph in self.visible(&visible) {
            let Some(text) = &self.blocks[paragraph].layout else {
                continue;
            };
            let top = self.top(paragraph);
            let layout = text.layout();
            let count = layout.len();
            for (index, line) in layout.lines().enumerate() {
                let metrics = line.metrics();
                let length = self.layout_text(paragraph).len();
                let range = line.text_range();
                runs.push(TextRun {
                    paragraph,
                    range: range.start.min(length)..range.end.min(length),
                    rect: Rect::new(
                        metrics.offset,
                        top + metrics.block_min_coord,
                        metrics.advance,
                        metrics.block_max_coord - metrics.block_min_coord,
                    ),
                    newline: index + 1 == count && paragraph + 1 < self.blocks.len(),
                });
            }
        }
        runs
    }

    pub(crate) fn text_selection(
        &self,
        owner: WidgetId,
        runs: &[TextRun],
    ) -> Option<TextSelection> {
        let locate = |position: Position| -> Option<TextPosition> {
            let index = self.layout_index(position);
            let (number, run) = runs.iter().enumerate().find(|(_, run)| {
                run.paragraph == position.paragraph
                    && run.range.start <= index
                    && (index < run.range.end || (index == run.range.end && !run.newline))
            })?;
            let text = self.layout_text(run.paragraph);
            let character_index = text[run.range.start..index.min(run.range.end)]
                .chars()
                .count();
            Some(TextPosition {
                node: owner.part(number as u64),
                character_index,
            })
        };
        Some(TextSelection {
            anchor: locate(self.selection.anchor)?,
            focus: locate(self.selection.focus)?,
        })
    }

    pub(crate) fn run_nodes(
        &self,
        owner: WidgetId,
        runs: &[TextRun],
        origin: Point,
    ) -> Vec<(NodeId, Node)> {
        runs.iter()
            .enumerate()
            .map(|(number, run)| {
                let text = self.layout_text(run.paragraph);
                let mut value = text[run.range.clone()].to_owned();
                if run.newline {
                    value.push('\n');
                }
                let mut part = Node::new(Role::TextRun);
                part.set_character_lengths(
                    value
                        .chars()
                        .map(|character| character.len_utf8() as u8)
                        .collect::<Vec<_>>(),
                );
                part.set_value(value);
                part.set_bounds(accesskit::Rect {
                    x0: f64::from(origin.x + run.rect.x),
                    y0: f64::from(origin.y + run.rect.y),
                    x1: f64::from(origin.x + run.rect.x + run.rect.width),
                    y1: f64::from(origin.y + run.rect.y + run.rect.height),
                });
                (owner.part(number as u64), part)
            })
            .collect()
    }
}
