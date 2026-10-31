use parley::Affinity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) struct Position {
    pub(crate) paragraph: usize,
    pub(crate) offset: usize,
}

impl Position {
    pub(super) const fn new(paragraph: usize, offset: usize) -> Self {
        Self { paragraph, offset }
    }

    pub(super) fn after(self, text: &str) -> Self {
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
pub(super) struct Selection {
    pub(super) anchor: Position,
    pub(super) focus: Position,
    pub(super) affinity: Affinity,
}

impl Selection {
    pub(super) fn caret(at: Position, affinity: Affinity) -> Self {
        Self {
            anchor: at,
            focus: at,
            affinity,
        }
    }

    pub(super) fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }

    pub(super) fn start(&self) -> Position {
        self.anchor.min(self.focus)
    }

    pub(super) fn end(&self) -> Position {
        self.anchor.max(self.focus)
    }
}
