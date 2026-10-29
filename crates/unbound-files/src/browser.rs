use std::io;
use std::path::{Path, PathBuf};

use crate::listing::{Entry, Kind, list};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub folder: PathBuf,
    pub entries: Vec<Entry>,
    pub selected: Option<usize>,
}

impl Column {
    fn open(folder: &Path, show_hidden: bool) -> Self {
        Self {
            folder: folder.to_path_buf(),
            entries: list(folder, show_hidden).unwrap_or_default(),
            selected: None,
        }
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        self.entries.get(self.selected?)
    }
}

#[derive(Debug)]
pub struct Browser {
    columns: Vec<Column>,
    active: usize,
    show_hidden: bool,
}

impl Browser {
    pub fn open(folder: &Path) -> io::Result<Self> {
        Ok(Self {
            columns: vec![Column {
                folder: folder.to_path_buf(),
                entries: list(folder, false)?,
                selected: None,
            }],
            active: 0,
            show_hidden: false,
        })
    }

    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn shows_hidden(&self) -> bool {
        self.show_hidden
    }

    pub fn selected(&self) -> Option<&Entry> {
        self.columns[self.active].selected_entry()
    }

    pub fn current_folder(&self) -> &Path {
        match self.selected() {
            Some(entry) if entry.kind == Kind::Folder => &entry.path,
            _ => &self.columns[self.active].folder,
        }
    }

    pub fn select(&mut self, column: usize, row: usize) {
        let Some(entry) = self
            .columns
            .get(column)
            .and_then(|shown| shown.entries.get(row))
            .cloned()
        else {
            return;
        };
        self.columns.truncate(column + 1);
        self.columns[column].selected = Some(row);
        self.active = column;
        if entry.kind == Kind::Folder {
            self.columns
                .push(Column::open(&entry.path, self.show_hidden));
        }
    }

    pub fn down(&mut self) {
        let column = &self.columns[self.active];
        let row = match column.selected {
            None => 0,
            Some(row) => (row + 1).min(column.entries.len().saturating_sub(1)),
        };
        self.select(self.active, row);
    }

    pub fn up(&mut self) {
        let column = &self.columns[self.active];
        let row = match column.selected {
            None => column.entries.len().saturating_sub(1),
            Some(row) => row.saturating_sub(1),
        };
        self.select(self.active, row);
    }

    pub fn right(&mut self) -> bool {
        let opened = self
            .selected()
            .is_some_and(|entry| entry.kind == Kind::Folder)
            && self
                .columns
                .get(self.active + 1)
                .is_some_and(|child| !child.entries.is_empty());
        if opened {
            self.select(self.active + 1, 0);
        }
        opened
    }

    pub fn left(&mut self) -> bool {
        if self.active == 0 {
            return false;
        }
        self.columns[self.active].selected = None;
        self.columns.truncate(self.active + 1);
        self.active -= 1;
        true
    }

    pub fn enclosing_folder(&mut self) -> bool {
        let root = self.columns[0].folder.clone();
        let Some(parent) = root.parent() else {
            return false;
        };
        let mut columns = vec![Column::open(parent, self.show_hidden)];
        columns.append(&mut self.columns);
        self.columns = columns;
        if let Some(row) = self.columns[0]
            .entries
            .iter()
            .position(|entry| entry.path == root)
        {
            self.columns[0].selected = Some(row);
        }
        self.active = self
            .columns
            .iter()
            .rposition(|column| column.selected.is_some())
            .unwrap_or(0);
        true
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        let chosen = self
            .columns
            .iter()
            .map(|column| column.selected_entry().map(|entry| entry.path.clone()))
            .collect::<Vec<_>>();
        let active = self.active;
        self.columns.truncate(1);
        self.columns[0] = Column::open(&self.columns[0].folder, self.show_hidden);
        self.active = 0;
        for (column, path) in chosen.into_iter().enumerate().take(active + 1) {
            let Some(row) = path.and_then(|path| {
                self.columns
                    .get(column)?
                    .entries
                    .iter()
                    .position(|entry| entry.path == path)
            }) else {
                break;
            };
            self.select(column, row);
        }
    }
}
