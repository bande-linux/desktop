use std::path::{Path, PathBuf};

use unbound_fs::{Entry, Sort, Trash, list, list_trash};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    Folder(PathBuf),
    Trash,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub location: Location,
    pub entries: Vec<Entry>,
    pub selected: Option<usize>,
    pub marked: Vec<usize>,
    pub anchor: Option<usize>,
    pub scroll: f32,
    pub filter: String,
}

impl Column {
    fn new(location: Location, entries: Vec<Entry>) -> Self {
        Self {
            location,
            entries,
            selected: None,
            marked: Vec::new(),
            anchor: None,
            scroll: 0.0,
            filter: String::new(),
        }
    }

    pub fn folder(&self) -> Option<&Path> {
        match &self.location {
            Location::Folder(folder) => Some(folder),
            Location::Trash => None,
        }
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        self.entries.get(self.selected?)
    }

    pub fn is_marked(&self, row: usize) -> bool {
        self.marked.contains(&row)
    }

    pub fn selection(&self) -> Vec<&Entry> {
        self.marked
            .iter()
            .filter_map(|row| self.entries.get(*row))
            .collect()
    }

    fn select_only(&mut self, row: usize) {
        self.selected = Some(row);
        self.anchor = Some(row);
        self.marked = vec![row];
    }

    fn clear(&mut self) {
        self.selected = None;
        self.anchor = None;
        self.marked.clear();
    }
}

#[derive(Debug)]
pub struct Browser {
    columns: Vec<Column>,
    active: usize,
    show_hidden: bool,
    trash: Trash,
    back: Vec<Location>,
    forward: Vec<Location>,
    sort: Sort,
    single: bool,
}

fn read(location: &Location, show_hidden: bool, trash: &Trash, sort: Sort) -> Vec<Entry> {
    match location {
        Location::Folder(folder) => list(folder, show_hidden, sort).unwrap_or_default(),
        Location::Trash => list_trash(trash, sort),
    }
}

impl Browser {
    pub fn open(location: Location, trash: Trash) -> Self {
        let entries = read(&location, false, &trash, Sort::default());
        Self {
            columns: vec![Column::new(location, entries)],
            active: 0,
            show_hidden: false,
            trash,
            back: Vec::new(),
            forward: Vec::new(),
            sort: Sort::default(),
            single: false,
        }
    }

    pub fn trash(&self) -> &Trash {
        &self.trash
    }

    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    pub fn columns_mut(&mut self) -> &mut [Column] {
        &mut self.columns
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn root(&self) -> &Location {
        &self.columns[0].location
    }

    pub fn shows_hidden(&self) -> bool {
        self.show_hidden
    }

    pub fn in_trash(&self) -> bool {
        self.columns[0].location == Location::Trash
    }

    pub fn active_column(&self) -> &Column {
        &self.columns[self.active]
    }

    pub fn selected(&self) -> Option<&Entry> {
        let column = self.active_column();
        (column.marked.len() <= 1)
            .then(|| column.selected_entry())
            .flatten()
    }

    pub fn selection(&self) -> Vec<&Entry> {
        self.active_column().selection()
    }

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.selection()
            .iter()
            .map(|entry| entry.path.clone())
            .collect()
    }

    pub fn current_location(&self) -> Location {
        match self.selected() {
            Some(entry) if entry.is_folder() => Location::Folder(entry.path.clone()),
            _ => self.active_column().location.clone(),
        }
    }

    pub fn current_folder(&self) -> Option<PathBuf> {
        match self.current_location() {
            Location::Folder(folder) => Some(folder),
            Location::Trash => None,
        }
    }

    pub fn active_folder(&self) -> Option<&Path> {
        self.active_column().folder()
    }

    pub fn go(&mut self, location: Location) {
        if *self.root() == location {
            return;
        }
        self.back.push(self.root().clone());
        self.forward.clear();
        self.show(location);
    }

    fn show(&mut self, location: Location) {
        let entries = read(&location, self.show_hidden, &self.trash, self.sort);
        self.columns = vec![Column::new(location, entries)];
        self.active = 0;
    }

    pub fn go_back(&mut self) -> bool {
        let Some(location) = self.back.pop() else {
            return false;
        };
        let current = self.root().clone();
        self.forward.push(current.clone());
        self.show(location);
        if let Location::Folder(folder) = current {
            self.reveal(&folder);
        }
        true
    }

    pub fn go_forward(&mut self) -> bool {
        let Some(location) = self.forward.pop() else {
            return false;
        };
        self.back.push(self.root().clone());
        self.show(location);
        true
    }

    pub fn sort(&self) -> Sort {
        self.sort
    }

    pub fn set_sort(&mut self, sort: Sort) {
        self.sort = sort;
        self.reload();
    }

    pub fn single(&self) -> bool {
        self.single
    }

    pub fn set_single(&mut self, single: bool) {
        self.single = single;
        if single {
            if let Some(folder) = self.active_folder().map(Path::to_path_buf) {
                let selection = self.selected_paths();
                if self.columns[0].folder() != Some(folder.as_path()) {
                    self.go(Location::Folder(folder));
                }
                if !selection.is_empty() {
                    self.reveal_all(&selection);
                }
            }
            self.columns.truncate(1);
            self.active = 0;
        } else {
            self.open_child(self.active);
        }
    }

    fn open_child(&mut self, column: usize) {
        self.columns.truncate(column + 1);
        if self.single {
            return;
        }
        let child = match self.columns[column].marked.as_slice() {
            [_] => self.columns[column]
                .selected_entry()
                .filter(|entry| entry.is_folder())
                .map(|entry| entry.path.clone()),
            _ => None,
        };
        if let Some(folder) = child {
            let location = Location::Folder(folder);
            let entries = read(&location, self.show_hidden, &self.trash, self.sort);
            self.columns.push(Column::new(location, entries));
        }
    }

    pub fn select(&mut self, column: usize, row: usize) {
        if self
            .columns
            .get(column)
            .is_none_or(|shown| row >= shown.entries.len())
        {
            return;
        }
        self.columns[column].select_only(row);
        self.active = column;
        self.open_child(column);
    }

    pub fn toggle(&mut self, column: usize, row: usize) {
        let Some(shown) = self.columns.get_mut(column) else {
            return;
        };
        if row >= shown.entries.len() {
            return;
        }
        if column != self.active {
            shown.clear();
        }
        match shown.marked.iter().position(|marked| *marked == row) {
            Some(index) => {
                shown.marked.remove(index);
                shown.selected = shown.marked.last().copied();
            }
            None => {
                shown.marked.push(row);
                shown.marked.sort_unstable();
                shown.selected = Some(row);
            }
        }
        shown.anchor = shown.selected;
        self.active = column;
        self.open_child(column);
    }

    pub fn extend_to(&mut self, column: usize, row: usize) {
        let Some(shown) = self.columns.get_mut(column) else {
            return;
        };
        if row >= shown.entries.len() {
            return;
        }
        let anchor = shown
            .anchor
            .filter(|_| column == self.active)
            .unwrap_or(row);
        shown.marked = (anchor.min(row)..=anchor.max(row)).collect();
        shown.selected = Some(row);
        shown.anchor = Some(anchor);
        self.active = column;
        self.open_child(column);
    }

    pub fn select_all(&mut self) {
        let column = &mut self.columns[self.active];
        if column.entries.is_empty() {
            return;
        }
        column.marked = (0..column.entries.len()).collect();
        column.anchor = Some(0);
        column.selected = Some(column.entries.len() - 1);
        self.open_child(self.active);
    }

    pub fn set_filter(&mut self, column: usize, filter: &str) {
        let Some(shown) = self.columns.get_mut(column) else {
            return;
        };
        if shown.filter == filter {
            return;
        }
        shown.filter = filter.to_owned();
        shown.scroll = 0.0;
        self.reload_column(column);
        let shown = &mut self.columns[column];
        if shown.selected.is_none() && !shown.entries.is_empty() && !filter.is_empty() {
            self.select(column, 0);
        }
    }

    pub fn focus_column(&mut self, column: usize) {
        if column >= self.columns.len() {
            return;
        }
        self.columns[column].clear();
        self.columns.truncate(column + 1);
        self.active = column;
    }

    fn next_row(&self, forward: bool) -> Option<usize> {
        let column = self.active_column();
        let last = column.entries.len().checked_sub(1)?;
        Some(match (column.selected, forward) {
            (None, true) => 0,
            (None, false) => last,
            (Some(row), true) => (row + 1).min(last),
            (Some(row), false) => row.saturating_sub(1),
        })
    }

    pub fn down(&mut self) {
        if let Some(row) = self.next_row(true) {
            self.select(self.active, row);
        }
    }

    pub fn up(&mut self) {
        if let Some(row) = self.next_row(false) {
            self.select(self.active, row);
        }
    }

    pub fn extend(&mut self, forward: bool) {
        if let Some(row) = self.next_row(forward) {
            self.extend_to(self.active, row);
        }
    }

    pub fn right(&mut self) -> bool {
        let opened = self.selected().is_some_and(Entry::is_folder)
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
        self.columns[self.active].clear();
        self.columns.truncate(self.active + 1);
        self.active -= 1;
        let column = &mut self.columns[self.active];
        if let Some(row) = column.selected {
            column.select_only(row);
        }
        true
    }

    pub fn enclosing_folder(&mut self) -> bool {
        let Some(root) = self.columns[0].folder().map(Path::to_path_buf) else {
            return false;
        };
        let Some(parent) = root.parent() else {
            return false;
        };
        if self.single {
            self.go(Location::Folder(parent.to_path_buf()));
            self.reveal(&root);
            return true;
        }
        let location = Location::Folder(parent.to_path_buf());
        let entries = read(&location, self.show_hidden, &self.trash, self.sort);
        let mut columns = vec![Column::new(location, entries)];
        columns.append(&mut self.columns);
        self.columns = columns;
        if let Some(row) = self.columns[0]
            .entries
            .iter()
            .position(|entry| entry.path == root)
        {
            self.columns[0].select_only(row);
        }
        self.active = self
            .columns
            .iter()
            .rposition(|column| column.selected.is_some())
            .unwrap_or(0);
        true
    }

    pub fn watched(&self) -> Vec<PathBuf> {
        self.columns
            .iter()
            .flat_map(|column| match &column.location {
                Location::Folder(folder) => vec![folder.clone()],
                Location::Trash => self.trash_info_dirs(),
            })
            .collect()
    }

    fn trash_info_dirs(&self) -> Vec<PathBuf> {
        vec![self.trash.home().root().join("info")]
    }

    fn reload_column(&mut self, index: usize) {
        let column = &self.columns[index];
        let selected_path = column.selected_entry().map(|entry| entry.path.clone());
        let marked_paths = column
            .selection()
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        let anchor_path = column
            .anchor
            .and_then(|row| column.entries.get(row))
            .map(|entry| entry.path.clone());
        let previous_row = column.selected;
        let mut entries = read(&column.location, self.show_hidden, &self.trash, self.sort);
        let filter = column.filter.to_lowercase();
        if !filter.is_empty() {
            entries.retain(|entry| entry.name.to_lowercase().contains(&filter));
        }
        let find = |path: &PathBuf| entries.iter().position(|entry| entry.path == *path);
        let marked = marked_paths.iter().filter_map(find).collect::<Vec<_>>();
        let selected = selected_path
            .as_ref()
            .and_then(find)
            .or_else(|| marked.last().copied());
        let anchor = anchor_path.as_ref().and_then(find).or(selected);
        let lost_selection = previous_row.is_some() && selected.is_none();
        let child_was = self
            .columns
            .get(index + 1)
            .map(|child| child.location.clone());
        let column = &mut self.columns[index];
        column.entries = entries;
        column.marked = marked;
        column.selected = selected;
        column.anchor = anchor;
        if lost_selection {
            self.columns.truncate(index + 1);
            if self.active > index {
                self.active = index;
            }
            if let Some(row) = previous_row
                .map(|row| row.min(self.columns[index].entries.len().saturating_sub(1)))
                .filter(|_| !self.columns[index].entries.is_empty())
                && self.active == index
            {
                self.columns[index].select_only(row);
                self.open_child(index);
            }
            return;
        }
        let child_now = match self.columns[index].marked.as_slice() {
            [_] => self.columns[index]
                .selected_entry()
                .filter(|entry| entry.is_folder())
                .map(|entry| Location::Folder(entry.path.clone())),
            _ => None,
        };
        if child_now != child_was {
            self.columns.truncate(index + 1);
            self.active = self.active.min(index);
            self.open_child(index);
        }
    }

    fn reload_where(&mut self, stale: impl Fn(&Location) -> bool) {
        let mut index = 0;
        while index < self.columns.len() {
            if stale(&self.columns[index].location) {
                self.reload_column(index);
            }
            index += 1;
        }
    }

    pub fn refresh(&mut self, changed: &[PathBuf]) {
        let trash_changed = self
            .trash_info_dirs()
            .iter()
            .any(|dir| changed.contains(dir));
        self.reload_where(|location| match location {
            Location::Folder(folder) => changed.contains(folder),
            Location::Trash => trash_changed,
        });
    }

    pub fn reload(&mut self) {
        self.reload_where(|_| true);
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.reload();
    }

    pub fn reveal(&mut self, path: &Path) {
        self.reveal_all(std::slice::from_ref(&path.to_path_buf()));
    }

    pub fn reveal_all(&mut self, paths: &[PathBuf]) {
        let Some(parent) = paths.first().and_then(|path| path.parent()) else {
            return;
        };
        let index = self
            .columns
            .iter()
            .position(|column| column.folder() == Some(parent));
        let index = match index {
            Some(index) => index,
            None => {
                let location = Location::Folder(parent.to_path_buf());
                if *self.root() != location {
                    self.back.push(self.root().clone());
                    self.forward.clear();
                }
                self.show(location);
                0
            }
        };
        self.columns.truncate(index + 1);
        self.reload_column(index);
        let column = &mut self.columns[index];
        let rows = paths
            .iter()
            .filter_map(|path| column.entries.iter().position(|entry| entry.path == *path))
            .collect::<Vec<_>>();
        let Some(last) = rows.last().copied() else {
            return;
        };
        column.marked = rows;
        column.marked.sort_unstable();
        column.selected = Some(last);
        column.anchor = Some(last);
        self.active = index;
        self.open_child(index);
    }
}
