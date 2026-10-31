use std::rc::Rc;

use bande_graphics::{Color, Cut, Image};

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub title: String,
    pub width: f32,
    pub numeric: bool,
    pub sortable: bool,
    pub flexible: bool,
}

impl Column {
    pub fn new(title: &str, width: f32) -> Self {
        Self {
            title: title.to_owned(),
            width,
            numeric: false,
            sortable: true,
            flexible: false,
        }
    }

    pub fn numeric(self) -> Self {
        Self {
            numeric: true,
            ..self
        }
    }

    pub fn unsortable(self) -> Self {
        Self {
            sortable: false,
            ..self
        }
    }

    pub fn flexible(self) -> Self {
        Self {
            flexible: true,
            ..self
        }
    }
}

#[derive(Debug, Clone)]
pub struct Cell {
    pub text: Rc<str>,
    pub icon: Option<Image>,
    pub muted: bool,
    pub tint: Option<Color>,
    pub label: bool,
    pub tabular: bool,
    pub cut: Cut,
}

impl Cell {
    pub fn new(text: impl Into<Rc<str>>) -> Self {
        Self {
            text: text.into(),
            icon: None,
            muted: false,
            tint: None,
            label: false,
            tabular: false,
            cut: Cut::End,
        }
    }

    pub fn icon(self, icon: Option<Image>) -> Self {
        Self { icon, ..self }
    }

    pub fn muted(self) -> Self {
        Self {
            muted: true,
            ..self
        }
    }

    pub fn tinted(self, tint: Color) -> Self {
        Self {
            tint: Some(tint),
            ..self
        }
    }

    pub fn label(self) -> Self {
        Self {
            label: true,
            ..self
        }
    }

    pub fn tabular(self) -> Self {
        Self {
            tabular: true,
            ..self
        }
    }

    pub fn cut_in_middle(self) -> Self {
        Self {
            cut: Cut::Middle,
            ..self
        }
    }
}

#[derive(Debug, Clone)]
pub struct Row {
    pub key: String,
    pub cells: Vec<Cell>,
    pub children: Vec<Row>,
    pub expandable: bool,
}

impl Row {
    pub fn new(key: impl Into<String>, cells: Vec<Cell>) -> Self {
        Self {
            key: key.into(),
            cells,
            children: Vec::new(),
            expandable: false,
        }
    }

    pub fn children(self, children: Vec<Row>) -> Self {
        Self { children, ..self }
    }

    pub fn expandable(self) -> Self {
        Self {
            expandable: true,
            ..self
        }
    }
}

#[derive(Debug)]
pub(super) struct Line {
    pub(super) key: String,
    pub(super) cells: Vec<Cell>,
    pub(super) depth: usize,
    pub(super) descendants: usize,
    pub(super) branch: bool,
}

impl Line {
    pub(super) fn unfilled(&self) -> bool {
        self.key.is_empty() && self.cells.is_empty()
    }
}

pub(super) fn flatten(rows: Vec<Row>, depth: usize, lines: &mut Vec<Line>) -> usize {
    let mut added = 0;
    for row in rows {
        let at = lines.len();
        let branch = row.expandable || !row.children.is_empty();
        lines.push(Line {
            key: row.key,
            cells: row.cells,
            depth,
            descendants: 0,
            branch,
        });
        let descendants = flatten(row.children, depth + 1, lines);
        lines[at].descendants = descendants;
        added += 1 + descendants;
    }
    added
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub column: usize,
    pub ascending: bool,
}
