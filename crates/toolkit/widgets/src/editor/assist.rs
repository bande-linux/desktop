use std::ops::Range;

use super::Editor;
use super::history::EditKind;
use super::selection::{Position, Selection};
use crate::spelling;
use parley::Affinity;

pub(super) struct Pending {
    pub(super) start: Position,
    pub(super) end: Position,
    pub(super) word: String,
}

pub(super) struct Corrected {
    pub(super) start: Position,
    pub(super) end: Position,
    pub(super) original: String,
    pub(super) fresh: bool,
}

const WORD_ENDERS: &str = ".,;:!?)";

impl Editor {
    fn apply_step(&mut self, start: Position, end: Position, text: &str) {
        let caret = self.selection.focus;
        let removed = self.slice(start, end).len();
        self.history.open = false;
        self.replace(start, end, text, EditKind::Replacing);
        self.history.open = false;
        let moved = if caret.paragraph == start.paragraph && caret.offset >= end.offset {
            Position::new(caret.paragraph, caret.offset + text.len() - removed)
        } else {
            caret
        };
        self.selection = Selection::caret(moved, Affinity::Downstream);
    }

    fn finished_word(&self) -> Option<(Position, Position)> {
        let caret = self.selection.focus;
        let text = &self.blocks[caret.paragraph].text[..caret.offset];
        let body = text.trim_end_matches(|character: char| {
            character.is_whitespace() || WORD_ENDERS.contains(character)
        });
        if body.len() == text.len() {
            return None;
        }
        let Range { start, end } = spelling::words(body).pop()?;
        (end == body.len()).then_some((
            Position::new(caret.paragraph, start),
            Position::new(caret.paragraph, end),
        ))
    }

    fn at_sentence_start(&self, start: Position) -> bool {
        let before = self.blocks[start.paragraph].text[..start.offset].trim_end();
        let enders = spelling::rule("sentence-enders");
        before.is_empty()
            || (before.len() < start.offset
                && before
                    .chars()
                    .last()
                    .is_some_and(|last| enders.contains(last)))
    }

    fn straighten(&mut self, typed: char) {
        if !spelling::option("smart-quotes") {
            return;
        }
        let caret = self.selection.focus;
        let Some(typed_at) = caret.offset.checked_sub(typed.len_utf8()) else {
            return;
        };
        let text = &self.blocks[caret.paragraph].text[..typed_at];
        let start = Position::new(caret.paragraph, typed_at);
        let opening = text.chars().last().is_none_or(|last| {
            last.is_whitespace() || "([{".contains(last) || spelling::rule("dash").contains(last)
        });
        let key = match (typed, opening) {
            ('"', true) => "quote-open",
            ('"', false) => "quote-close",
            ('\'', true) => "apostrophe-open",
            ('\'', false) => "apostrophe-close",
            ('-', _) if text.ends_with('-') => "dash",
            _ => return,
        };
        let replacement = spelling::rule(key);
        if replacement.is_empty() {
            return;
        }
        let from = if typed == '-' {
            Position::new(caret.paragraph, typed_at - 1)
        } else {
            start
        };
        self.apply_step(from, caret, &replacement);
    }

    fn double_space(&mut self) {
        let caret = self.selection.focus;
        let text = &self.blocks[caret.paragraph].text[..caret.offset];
        let period = spelling::rule("double-space");
        if !spelling::option("double-space") || period.is_empty() {
            return;
        }
        let Some(before) = text.strip_suffix("  ") else {
            return;
        };
        if before.chars().last().is_some_and(char::is_alphanumeric) {
            let start = Position::new(caret.paragraph, before.len());
            self.apply_step(start, caret, &format!("{period} "));
        }
    }

    pub(super) fn after_typing(&mut self, typed: char) {
        if !self.spelling || self.read_only || self.preedit.is_some() {
            return;
        }
        spelling::ensure_rules();
        if let Some(corrected) = &mut self.corrected {
            corrected.fresh = false;
        }
        self.straighten(typed);
        if typed == ' ' {
            self.double_space();
        }
        if !(typed.is_whitespace() || WORD_ENDERS.contains(typed)) {
            return;
        }
        let Some((start, end)) = self.finished_word() else {
            return;
        };
        let word = self.slice(start, end);
        if let Some(expansion) = spelling::replacement(&word) {
            self.apply_step(start, end, &expansion);
            return;
        }
        let capitalised = spelling::option("capitalise") && self.at_sentence_start(start);
        if capitalised && word.chars().next().is_some_and(char::is_lowercase) {
            let mut characters = word.chars();
            let upper: String = characters
                .next()
                .into_iter()
                .flat_map(char::to_uppercase)
                .chain(characters)
                .collect();
            let end = Position::new(start.paragraph, start.offset + upper.len());
            self.apply_step(
                start,
                Position::new(start.paragraph, start.offset + word.len()),
                &upper,
            );
            spelling::ask_correction(&upper);
            self.pending = Some(Pending {
                start,
                end,
                word: upper,
            });
            return;
        }
        spelling::ask_correction(&word);
        self.pending = Some(Pending { start, end, word });
    }

    pub(crate) fn spelling_answered(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let Some(correction) = spelling::correction(&pending.word) else {
            self.pending = Some(pending);
            return;
        };
        if correction.is_empty()
            || pending.end.paragraph >= self.blocks.len()
            || self.blocks[pending.end.paragraph]
                .text
                .get(pending.start.offset..pending.end.offset)
                != Some(pending.word.as_str())
        {
            return;
        }
        self.apply_step(pending.start, pending.end, &correction);
        self.corrected = Some(Corrected {
            start: pending.start,
            end: Position::new(
                pending.start.paragraph,
                pending.start.offset + correction.len(),
            ),
            original: pending.word,
            fresh: true,
        });
    }

    pub(super) fn change_back(&mut self) -> bool {
        let Some(corrected) = self.corrected.take() else {
            return false;
        };
        let intact = self
            .blocks
            .get(corrected.start.paragraph)
            .is_some_and(|block| {
                block.text.is_char_boundary(corrected.start.offset)
                    && block.text.is_char_boundary(corrected.end.offset)
            });
        if !intact {
            return false;
        }
        self.apply_step(corrected.start, corrected.end, &corrected.original);
        spelling::changed_back(&corrected.original);
        true
    }

    pub(super) fn backspace_restores(&mut self) -> bool {
        self.corrected
            .as_ref()
            .is_some_and(|corrected| corrected.fresh)
            && self.change_back()
    }
}
