use parley::{Affinity, Selection as Span};
use unbound_graphics::{Point, TextSystem};

use super::history::EditKind;
use super::selection::{Position, Selection};
use super::{Editor, Preedit};
use crate::{ClipboardRequest, Context, Key, Modifiers};

impl Editor {
    pub(super) fn go(&mut self, to: (Position, Affinity), extend: bool) {
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

    pub(super) fn end_composition(&mut self, cx: &mut Context) {
        if self.preedit.is_some() {
            self.finish_composition();
            cx.input_method_reset = true;
        }
    }

    fn finish_composition(&mut self) {
        if let Some(preedit) = self.preedit.take() {
            self.invalidate_paragraph(self.selection.focus.paragraph);
            self.replace_selection(&preedit.text, EditKind::Replacing);
            self.history.open = false;
        }
    }
}
