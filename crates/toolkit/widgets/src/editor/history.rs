use parley::Affinity;

use super::Editor;
use super::selection::{Position, Selection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EditKind {
    Typing,
    Deleting,
    Replacing,
}

#[derive(Debug, Clone)]
pub(super) struct Edit {
    pub(super) start: Position,
    pub(super) removed: String,
    pub(super) inserted: String,
    pub(super) before: Selection,
    pub(super) kind: EditKind,
}

#[derive(Default)]
pub(super) struct History {
    pub(super) undo: Vec<Edit>,
    redo: Vec<Edit>,
    pub(super) open: bool,
}

impl History {
    pub(super) fn record(&mut self, edit: Edit) {
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

pub(super) fn coalesce(last: &mut Edit, edit: &Edit) -> bool {
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

impl Editor {
    pub(crate) fn can_undo(&self) -> bool {
        !self.history.undo.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.history.redo.is_empty()
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
}
