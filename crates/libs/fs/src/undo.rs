use crate::ops::Change;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Copy,
    Move,
    Duplicate,
    Rename,
    NewFolder,
    NewDocument,
    Trash,
    PutBack,
    Compress,
    Extract,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub action: Action,
    pub changes: Vec<Change>,
}

#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
}

impl History {
    pub fn record(&mut self, action: Action, changes: Vec<Change>) {
        if !changes.is_empty() {
            self.undo.push(Step { action, changes });
            self.redo.clear();
        }
    }

    pub fn next_undo(&self) -> Option<&Step> {
        self.undo.last()
    }

    pub fn next_redo(&self) -> Option<&Step> {
        self.redo.last()
    }

    pub fn take_undo(&mut self) -> Option<Step> {
        self.undo.pop()
    }

    pub fn take_redo(&mut self) -> Option<Step> {
        self.redo.pop()
    }

    pub fn undone(&mut self, step: Step) {
        if !step.changes.is_empty() {
            self.redo.push(step);
        }
    }

    pub fn redone(&mut self, step: Step) {
        if !step.changes.is_empty() {
            self.undo.push(step);
        }
    }
}
