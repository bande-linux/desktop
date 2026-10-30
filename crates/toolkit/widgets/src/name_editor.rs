use std::ops::Range;

use super::{Key, Modifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameEditor {
    text: String,
    cursor: usize,
    anchor: usize,
}

impl NameEditor {
    pub fn new(name: &str, is_folder: bool) -> Self {
        let stem_end = match name.rfind('.') {
            Some(dot) if dot > 0 && !is_folder => dot,
            _ => name.len(),
        };
        Self {
            text: name.to_owned(),
            cursor: stem_end,
            anchor: 0,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn selection(&self) -> Range<usize> {
        self.cursor.min(self.anchor)..self.cursor.max(self.anchor)
    }

    fn previous(&self, at: usize) -> usize {
        self.text[..at]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next(&self, at: usize) -> usize {
        self.text[at..]
            .chars()
            .next()
            .map_or(at, |character| at + character.len_utf8())
    }

    fn move_to(&mut self, at: usize, extend: bool) {
        self.cursor = at;
        if !extend {
            self.anchor = at;
        }
    }

    pub fn insert(&mut self, text: &str) {
        let range = self.selection();
        self.text.replace_range(range.clone(), text);
        self.move_to(range.start + text.len(), false);
    }

    fn delete(&mut self, forward: bool) {
        if self.selection().is_empty() {
            let other = if forward {
                self.next(self.cursor)
            } else {
                self.previous(self.cursor)
            };
            self.anchor = other;
        }
        self.insert("");
    }

    pub fn key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        let extend = modifiers.shift;
        let collapse = |editor: &Self, forward: bool| {
            let range = editor.selection();
            if forward { range.end } else { range.start }
        };
        match key {
            Key::Left if modifiers.logo => self.move_to(0, extend),
            Key::Right if modifiers.logo => self.move_to(self.text.len(), extend),
            Key::Left if !extend && !self.selection().is_empty() => {
                self.move_to(collapse(self, false), false);
            }
            Key::Right if !extend && !self.selection().is_empty() => {
                self.move_to(collapse(self, true), false);
            }
            Key::Left => self.move_to(self.previous(self.cursor), extend),
            Key::Right => self.move_to(self.next(self.cursor), extend),
            Key::Home | Key::Up => self.move_to(0, extend),
            Key::End | Key::Down => self.move_to(self.text.len(), extend),
            Key::Backspace => self.delete(false),
            Key::Delete => self.delete(true),
            Key::Character('a') if modifiers.logo => {
                self.anchor = 0;
                self.cursor = self.text.len();
            }
            Key::Character(character) if !modifiers.logo && !modifiers.control => {
                self.insert(character.encode_utf8(&mut [0; 4]));
            }
            Key::Space if !modifiers.logo => self.insert(" "),
            _ => return false,
        }
        true
    }
}
