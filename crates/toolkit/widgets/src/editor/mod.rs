mod history;
mod keys;
mod layout;
mod selection;

use std::borrow::Cow;
use std::ops::Range;

use accesskit::{
    Action, ActionData, ActionRequest, Node, NodeId, Role, TextPosition, TextSelection,
};
use bande_graphics::{Color, Item, Paint, Point, Rect, Shape, TextSystem};
use parley::{Affinity, Cursor, Selection as Span};

use self::history::{Edit, EditKind, History};
use self::layout::{Block, bounds};
use self::selection::{Position, Selection};
use super::{
    ClipboardRequest, Context, DragAction, Event, Key, Menu, MenuItem, Modifiers, PointerButton,
    WidgetId,
};
use crate::L10N;

const CARET_WIDTH: f32 = 1.5;
const LINE_HEIGHT_RATIO: f32 = 1.21;
const UNDERLINE: f32 = 1.0;
const CLAUSE_UNDERLINE: f32 = 2.0;
const DRAG_START: f32 = 4.0;

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
enum Carry {
    Pending(Point),
    Dragging(usize, usize),
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
    scale: f32,
    multi_line: bool,
    width: Option<f32>,
    color: Color,
    line_height: f32,
    drag: Option<Drag>,
    drag_point: Option<Point>,
    carry: Option<Carry>,
    drop_caret: Option<(Position, Affinity)>,
    reveal: bool,
    read_only: bool,
    page: f32,
    frame: u64,
    laid_out: usize,
    revision: u64,
    offered: Option<(u64, Range<usize>)>,
}

fn normalized(text: &str) -> Cow<'_, str> {
    if text.contains('\r') {
        Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(text)
    }
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
            scale: 1.0,
            multi_line,
            width: None,
            color: Color::BLACK,
            line_height: (font_size * LINE_HEIGHT_RATIO).ceil(),
            drag: None,
            drag_point: None,
            carry: None,
            drop_caret: None,
            reveal: false,
            read_only: false,
            page: 0.0,
            frame: 0,
            laid_out: 0,
            revision: 0,
            offered: None,
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

    pub(crate) fn scale(&self) -> f32 {
        self.scale
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

    pub(crate) fn set_width(&mut self, width: Option<f32>) {
        if self.width == width {
            return;
        }
        self.width = width;
        self.invalidate();
    }

    pub(crate) fn set_scale(&mut self, scale: f32) {
        if self.scale == scale {
            return;
        }
        self.scale = scale;
        self.line_height = (self.font_size * LINE_HEIGHT_RATIO * scale).ceil();
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

    fn press(&mut self, cx: &mut Context, point: Point, count: u8, extend: bool) {
        self.end_composition(cx);
        self.history.open = false;
        self.goal_x = None;
        self.carry = None;
        if count <= 1 && !extend && self.on_selection(&mut cx.text, point) {
            self.carry = Some(Carry::Pending(point));
            return;
        }
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

    fn on_selection(&mut self, text: &mut TextSystem, point: Point) -> bool {
        if self.selection.is_collapsed() {
            return false;
        }
        let (at, _) = self.position_at(text, point);
        self.selection.start() <= at && at < self.selection.end()
    }

    fn carry_to(&mut self, cx: &mut Context, point: Point) -> bool {
        let Some(Carry::Pending(from)) = self.carry else {
            return false;
        };
        if (point.x - from.x).hypot(point.y - from.y) <= DRAG_START {
            return true;
        }
        let range = self.selection();
        self.carry = Some(Carry::Dragging(range.start, range.end));
        cx.start_text_drag(self.selected_text());
        true
    }

    fn put_down(&mut self, text: &mut TextSystem, point: Point) -> bool {
        if !matches!(self.carry, Some(Carry::Pending(_))) {
            return false;
        }
        self.carry = None;
        let (at, affinity) = self.position_at(text, point);
        self.selection = Selection::caret(at, affinity);
        true
    }

    fn dragged_range(&self) -> Option<Range<usize>> {
        match self.carry {
            Some(Carry::Dragging(start, end)) => Some(start..end),
            _ => None,
        }
    }

    pub(crate) fn drop_target(
        &mut self,
        text: &mut TextSystem,
        point: Option<Point>,
    ) -> Option<DragAction> {
        self.drop_caret = None;
        let point = point.filter(|_| !self.read_only)?;
        let (at, affinity) = self.position_at(text, point);
        let offset = self.byte_offset(at);
        let dragged = self.dragged_range();
        if dragged
            .as_ref()
            .is_some_and(|range| range.start < offset && offset < range.end)
        {
            return None;
        }
        self.drop_caret = Some((at, affinity));
        Some(match dragged {
            Some(_) => DragAction::Move,
            None => DragAction::Copy,
        })
    }

    pub(crate) fn drop_text(
        &mut self,
        text: &mut TextSystem,
        point: Point,
        dropped: &str,
        action: DragAction,
    ) {
        self.drop_caret = None;
        let (at, _) = self.position_at(text, point);
        let mut offset = self.byte_offset(at);
        if action == DragAction::Move
            && let Some(range) = self.dragged_range()
        {
            self.carry = None;
            if range.start < offset && offset < range.end {
                return;
            }
            self.select(range.clone());
            self.insert("");
            if offset >= range.end {
                offset -= range.end - range.start;
            }
        }
        self.select(offset..offset);
        self.insert(dropped);
        let end = self.selection().end;
        self.select(offset..end);
        self.reveal = true;
    }

    pub(crate) fn drag_ended(&mut self, action: Option<DragAction>) {
        let dragged = self.dragged_range();
        self.carry = None;
        self.drop_caret = None;
        if let Some(range) = dragged
            && action == Some(DragAction::Move)
            && !self.read_only
        {
            self.select(range);
            self.insert("");
        }
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
        let handled = self.handle_event(cx, event, origin);
        self.offer_primary(cx);
        handled
    }

    fn offer_primary(&mut self, cx: &mut Context) {
        if self.selection.is_collapsed() || self.drag.is_some() || cx.clipboard.is_some() {
            return;
        }
        let offered = (self.revision, self.selection());
        if self.offered.as_ref() == Some(&offered) {
            return;
        }
        self.offered = Some(offered);
        cx.clipboard = Some(ClipboardRequest::SelectPrimary(self.selected_text()));
    }

    fn paste_primary(&mut self, cx: &mut Context, point: Point) -> bool {
        if self.read_only {
            return false;
        }
        self.end_composition(cx);
        self.history.open = false;
        let at = self.position_at(&mut cx.text, point);
        self.go(at, false);
        cx.clipboard = Some(ClipboardRequest::PastePrimary);
        true
    }

    fn handle_event(&mut self, cx: &mut Context, event: &Event, origin: Point) -> bool {
        let local = |point: Point| Point::new(point.x - origin.x, point.y - origin.y);
        match event {
            Event::PointerDown(pointer) if pointer.button == PointerButton::Middle => {
                return self.paste_primary(cx, local(pointer.position));
            }
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
                self.end_composition(cx);
                self.place_caret_for_menu(&mut cx.text, local(pointer.position));
            }
            Event::PointerLeave => {
                self.drag = None;
                self.drag_point = None;
                if matches!(self.carry, Some(Carry::Pending(_))) {
                    self.carry = None;
                }
                return false;
            }
            Event::PointerMove(pointer) if matches!(self.carry, Some(Carry::Pending(_))) => {
                return self.carry_to(cx, local(pointer.position));
            }
            Event::PointerUp(pointer)
                if pointer.is_primary() && self.put_down(&mut cx.text, local(pointer.position)) =>
            {
                self.drag_point = None;
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

    fn set_text_selection(
        &mut self,
        runs: &[TextRun],
        selection: &TextSelection,
        owner: WidgetId,
    ) -> bool {
        if self.preedit.is_some() {
            return false;
        }
        let locate = |position: &TextPosition| {
            let index = WidgetId::owner_of_part(position.node)
                .filter(|(id, _)| *id == owner)
                .map(|(_, part)| part as usize)?;
            let run = runs.get(index)?;
            let text = self.layout_text(run.paragraph);
            let offset = text
                .get(run.range.clone())?
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
                item("copy", "Ctrl+C", selected),
                MenuItem::separator(),
                item("select-all", "Ctrl+A", !self.is_empty()),
            ]);
        }
        Menu::new(vec![
            item("undo", "Ctrl+Z", self.can_undo()),
            item("redo", "Shift+Ctrl+Z", self.can_redo()),
            MenuItem::separator(),
            item("cut", "Ctrl+X", selected),
            item("copy", "Ctrl+C", selected),
            item("paste", "Ctrl+V", true),
            MenuItem::separator(),
            item("select-all", "Ctrl+A", !self.is_empty()),
            MenuItem::separator(),
            item("emoji-and-symbols", "Super+.", true),
        ])
    }

    pub(crate) fn menu_action(&mut self, cx: &mut Context, action: &str) -> bool {
        let shortcut = |character: char, shift: bool| {
            (
                Key::Character(character),
                Modifiers {
                    shift,
                    control: true,
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
        let handled = self.key(cx, key, modifiers);
        self.offer_primary(cx);
        handled
    }

    fn place_caret_for_menu(&mut self, text: &mut TextSystem, point: Point) {
        let (at, affinity) = self.position_at(text, point);
        if self.selection.is_collapsed() || at < self.selection.start() || at > self.selection.end()
        {
            self.go((at, affinity), false);
        }
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
            (true, _) => cx.palette.accent_soft,
            (false, true) => cx.palette.well,
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
        if focused && !self.read_only && self.selection.is_collapsed() && !caret_hidden {
            let mut index = self.layout_index(self.selection.focus);
            if let Some(Preedit {
                cursor: Some((begin, _)),
                ..
            }) = &self.preedit
            {
                index = self.selection.focus.offset + begin;
            }
            let focus = self.selection.focus.paragraph;
            items.extend(self.caret_item(cx, origin, focus, index, self.selection.affinity));
        }
        if let Some((at, affinity)) = self.drop_caret {
            let index = self.layout_index(at);
            items.extend(self.caret_item(cx, origin, at.paragraph, index, affinity));
        }
    }

    fn caret_item(
        &self,
        cx: &Context,
        origin: Point,
        paragraph: usize,
        index: usize,
        affinity: Affinity,
    ) -> Option<Item> {
        let layout = self.blocks.get(paragraph)?.layout.as_ref()?.layout();
        let caret =
            bounds(Cursor::from_byte_index(layout, index, affinity).geometry(layout, CARET_WIDTH));
        Some(Item::Fill {
            shape: Shape::rect(Rect::new(
                origin.x + caret.x,
                origin.y + self.top(paragraph) + caret.y,
                CARET_WIDTH,
                caret.height,
            )),
            paint: cx.palette.accent_text.into(),
        })
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
            let length = self.layout_text(paragraph).len();
            for (index, line) in layout.lines().enumerate() {
                let metrics = line.metrics();
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
                    && (run.range.start..=run.range.end).contains(&index)
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
