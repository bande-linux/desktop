use std::collections::{BTreeSet, HashMap, HashSet};
use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use bande_fs::{
    Entry, FolderChanges, Listing, Sort, Trash, compare, is_hidden, list, list_trash, merge, sort,
};

use crate::lister::{FIRST_CHUNK, Lister, Ticket};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    Folder(PathBuf),
    Trash,
    Recents,
    Search(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub location: Location,
    pub entries: Vec<Entry>,
    pub selected: Option<usize>,
    pub marked: Vec<usize>,
    pub anchor: Option<usize>,
    pub revision: u64,
    pub edits: Vec<RowEdit>,
    pub depths: Vec<u8>,
    pub denied: bool,
    listing: Option<Ticket>,
    awaited: Vec<PathBuf>,
}

struct Contents {
    entries: Vec<Entry>,
    listing: Option<Ticket>,
    denied: bool,
}

impl Contents {
    fn listed(entries: Vec<Entry>) -> Self {
        Self {
            entries,
            listing: None,
            denied: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowEdit {
    Insert(usize),
    Remove(usize),
}

const MAX_EDITS: usize = 512;
const MAX_EXPANDED: usize = 1_000;

fn next_revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Column {
    fn new(location: Location, contents: Contents) -> Self {
        Self {
            revision: next_revision(),
            edits: Vec::new(),
            depths: Vec::new(),
            denied: contents.denied,
            location,
            entries: contents.entries,
            selected: None,
            marked: Vec::new(),
            anchor: None,
            listing: contents.listing,
            awaited: Vec::new(),
        }
    }

    pub fn is_listing(&self) -> bool {
        self.listing.is_some()
    }

    pub fn is_nested(&self) -> bool {
        !self.depths.is_empty()
    }

    fn replaced(&mut self) {
        self.revision = next_revision();
        self.edits.clear();
    }

    fn remove_row(&mut self, row: usize) {
        self.entries.remove(row);
        self.edits.push(RowEdit::Remove(row));
        let shift = |at: usize| match at.cmp(&row) {
            std::cmp::Ordering::Less => Some(at),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(at - 1),
        };
        self.marked = self.marked.iter().filter_map(|at| shift(*at)).collect();
        self.selected = self.selected.and_then(shift);
        self.anchor = self.anchor.and_then(shift);
    }

    fn insert_row(&mut self, entry: Entry, sort: Sort) -> usize {
        let row = self
            .entries
            .partition_point(|shown| compare(shown, &entry, sort) == std::cmp::Ordering::Less);
        self.entries.insert(row, entry);
        self.edits.push(RowEdit::Insert(row));
        let shift = |at: usize| if at >= row { at + 1 } else { at };
        for at in &mut self.marked {
            *at = shift(*at);
        }
        self.selected = self.selected.map(shift);
        self.anchor = self.anchor.map(shift);
        row
    }

    fn patch(&mut self, path: &Path, sort: Sort, show_hidden: bool) {
        if self.edits.len() >= MAX_EDITS {
            self.replaced();
        }
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            return;
        };
        let old = self.entries.iter().position(|entry| entry.name == name);
        let fresh = ((show_hidden || !is_hidden(&name)) && path.symlink_metadata().is_ok())
            .then(|| Entry::read(path.to_path_buf(), name));
        if let (Some(row), Some(entry)) = (old, &fresh)
            && self.entries[row] == *entry
        {
            return;
        }
        if old.is_none() && fresh.is_none() {
            return;
        }
        let was = old.map(|row| {
            (
                self.is_marked(row),
                self.selected == Some(row),
                self.anchor == Some(row),
            )
        });
        if let Some(row) = old {
            self.remove_row(row);
        }
        if let Some(entry) = fresh {
            let row = self.insert_row(entry, sort);
            if let Some((marked, selected, anchor)) = was {
                if marked && let Err(at) = self.marked.binary_search(&row) {
                    self.marked.insert(at, row);
                }
                if selected {
                    self.selected = Some(row);
                }
                if anchor {
                    self.anchor = Some(row);
                }
            }
        }
    }

    fn resort(&mut self, sort: Sort) {
        let mut order = (0..self.entries.len()).collect::<Vec<_>>();
        order.sort_by(|a, b| compare(&self.entries[*a], &self.entries[*b], sort));
        let mut moved = vec![0; order.len()];
        for (row, was) in order.iter().enumerate() {
            moved[*was] = row;
        }
        let mut entries = std::mem::take(&mut self.entries)
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();
        self.entries = order
            .iter()
            .filter_map(|was| entries.get_mut(*was).and_then(Option::take))
            .collect();
        let to = |row: usize| moved.get(row).copied();
        self.marked = self.marked.iter().filter_map(|row| to(*row)).collect();
        self.marked.sort_unstable();
        self.selected = self.selected.and_then(to);
        self.anchor = self.anchor.and_then(to);
        self.replaced();
    }

    fn merge(&mut self, chunk: Vec<Entry>, sort: Sort) -> bool {
        let moved = merge(&mut self.entries, chunk, sort);
        let to = |row: usize| moved.get(row).copied();
        self.marked = self.marked.iter().filter_map(|row| to(*row)).collect();
        self.selected = self.selected.and_then(to);
        self.anchor = self.anchor.and_then(to);
        self.replaced();
        if self.awaited.is_empty() {
            return false;
        }
        let awaited = self.awaited.iter().collect::<HashSet<_>>();
        let arrived = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| awaited.contains(&entry.path))
            .map(|(row, entry)| (row, entry.path.clone()))
            .collect::<Vec<_>>();
        if arrived.is_empty() {
            return false;
        }
        self.awaited
            .retain(|path| arrived.iter().all(|(_, found)| found != path));
        self.marked.extend(arrived.iter().map(|(row, _)| *row));
        self.marked.sort_unstable();
        self.marked.dedup();
        self.selected = arrived.last().map(|(row, _)| *row);
        self.anchor = self.selected;
        true
    }

    pub fn folder(&self) -> Option<&Path> {
        match &self.location {
            Location::Folder(folder) => Some(folder),
            _ => None,
        }
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        self.entries.get(self.selected?)
    }

    pub fn is_marked(&self, row: usize) -> bool {
        self.marked.binary_search(&row).is_ok()
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
    found: Vec<PathBuf>,
    recents: Vec<PathBuf>,
    lister: Option<Lister>,
    nesting: bool,
    expanded: BTreeSet<PathBuf>,
}

fn entries(paths: &[PathBuf]) -> Vec<Entry> {
    paths
        .iter()
        .filter(|path| path.symlink_metadata().is_ok())
        .filter_map(|path| {
            let name = path.file_name()?.to_string_lossy().into_owned();
            Some(Entry::read(path.clone(), name))
        })
        .collect()
}

impl Browser {
    pub fn open(location: Location, trash: Trash) -> Self {
        let mut browser = Self {
            columns: Vec::new(),
            active: 0,
            show_hidden: false,
            trash,
            back: Vec::new(),
            forward: Vec::new(),
            sort: Sort::default(),
            single: false,
            found: Vec::new(),
            recents: Vec::new(),
            lister: Lister::new()
                .inspect_err(|error| {
                    tracing::warn!("cannot list folders in the background: {error}")
                })
                .ok(),
            nesting: false,
            expanded: BTreeSet::new(),
        };
        browser.show(location);
        browser
    }

    fn read(&mut self, location: &Location) -> Contents {
        match location {
            Location::Folder(folder) => self.read_folder(folder),
            Location::Trash => Contents::listed(list_trash(&self.trash, self.order())),
            Location::Recents => Contents::listed(entries(&self.recents)),
            Location::Search(_) => {
                let mut found = entries(&self.found);
                sort(&mut found, self.order());
                Contents::listed(found)
            }
        }
    }

    fn read_folder(&mut self, folder: &Path) -> Contents {
        let mut listing = match Listing::open(folder, self.show_hidden) {
            Ok(listing) => listing,
            Err(error) => {
                return Contents {
                    denied: error.kind() == io::ErrorKind::PermissionDenied,
                    ..Contents::listed(Vec::new())
                };
            }
        };
        let first = match self.lister {
            Some(_) => FIRST_CHUNK,
            None => usize::MAX,
        };
        let mut entries = listing.next_chunk(first);
        let order = self.order();
        sort(&mut entries, order);
        let listing = match &mut self.lister {
            Some(lister) if !listing.is_done() => Some(lister.finish(listing, order)),
            _ => None,
        };
        Contents {
            entries,
            listing,
            denied: false,
        }
    }

    pub fn listing_fd(&self) -> Option<BorrowedFd<'_>> {
        self.lister.as_ref().map(Lister::fd)
    }

    pub fn take_listed(&mut self) -> (bool, bool) {
        let Some(lister) = &self.lister else {
            return (false, false);
        };
        let mut changed = false;
        let mut selected = false;
        let order = self.order();
        for listed in lister.take() {
            let Some(index) = self
                .columns
                .iter()
                .position(|column| column.listing.as_deref() == Some(&listed.id))
            else {
                continue;
            };
            let column = &mut self.columns[index];
            changed = true;
            if column.merge(listed.entries, order) {
                selected = true;
                if self.active <= index {
                    self.active = index;
                    self.open_child(index);
                }
            }
            if listed.done {
                let column = &mut self.columns[index];
                column.listing = None;
                column.awaited.clear();
                if index == 0 {
                    self.nest_root();
                }
            }
        }
        (changed, selected)
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub fn searching(&self) -> bool {
        matches!(self.root(), Location::Search(_))
    }

    pub fn set_found(&mut self, found: Vec<PathBuf>) {
        self.found = found;
        self.reload_where(|location| matches!(location, Location::Search(_)));
    }

    pub fn set_recents(&mut self, recents: Vec<PathBuf>) {
        if recents != self.recents {
            self.recents = recents;
            self.reload_where(|location| *location == Location::Recents);
        }
    }

    pub fn replace(&mut self, location: Location) {
        self.show(location);
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
            _ => None,
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
        let entries = self.read(&location);
        self.columns = vec![Column::new(location, entries)];
        self.active = 0;
        self.nest_root();
    }

    fn nests(&self) -> bool {
        self.nesting && self.single && !self.expanded.is_empty()
    }

    fn nest_into(
        &self,
        roots: Vec<Entry>,
        depth: u8,
        entries: &mut Vec<Entry>,
        depths: &mut Vec<u8>,
    ) {
        for entry in roots {
            let open = entry.is_folder() && self.expanded.contains(&entry.path);
            let folder = entry.path.clone();
            entries.push(entry);
            depths.push(depth);
            if open {
                let children =
                    list(&folder, self.show_hidden, self.children_order()).unwrap_or_default();
                self.nest_into(children, depth.saturating_add(1), entries, depths);
            }
        }
    }

    fn nested(&self, roots: Vec<Entry>) -> (Vec<Entry>, Vec<u8>) {
        if !self.nests() {
            return (roots, Vec::new());
        }
        let mut entries = Vec::with_capacity(roots.len());
        let mut depths = Vec::with_capacity(roots.len());
        self.nest_into(roots, 0, &mut entries, &mut depths);
        if depths.iter().all(|depth| *depth == 0) {
            depths.clear();
        }
        (entries, depths)
    }

    fn nest_root(&mut self) {
        let Some(column) = self.columns.first() else {
            return;
        };
        if column.folder().is_none()
            || column.is_listing()
            || (!column.is_nested() && !self.nests())
        {
            return;
        }
        let column = &mut self.columns[0];
        let marked = column
            .selection()
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        let selected = column.selected_entry().map(|entry| entry.path.clone());
        let anchor = column
            .anchor
            .and_then(|row| column.entries.get(row))
            .map(|entry| entry.path.clone());
        let entries = std::mem::take(&mut column.entries);
        let depths = std::mem::take(&mut column.depths);
        let mut roots = entries
            .into_iter()
            .zip(depths.into_iter().chain(std::iter::repeat(0)))
            .filter(|(_, depth)| *depth == 0)
            .map(|(entry, _)| entry)
            .collect::<Vec<_>>();
        sort(&mut roots, self.order());
        let (entries, depths) = self.nested(roots);
        let rows = entries
            .iter()
            .enumerate()
            .map(|(row, entry)| (entry.path.clone(), row))
            .collect::<HashMap<_, _>>();
        let find = |path: &PathBuf| rows.get(path).copied();
        let column = &mut self.columns[0];
        column.marked = marked.iter().filter_map(find).collect();
        column.marked.sort_unstable();
        column.selected = selected.as_ref().and_then(find);
        column.anchor = anchor.as_ref().and_then(find).or(column.selected);
        column.entries = entries;
        column.depths = depths;
        column.replaced();
    }

    pub fn set_nesting(&mut self, nesting: bool) {
        if self.nesting != nesting {
            self.nesting = nesting;
            self.nest_root();
        }
    }

    pub fn is_expanded(&self, folder: &Path) -> bool {
        self.expanded.contains(folder)
    }

    pub fn set_expanded(&mut self, folders: &[PathBuf], expanded: bool, all: bool) {
        if !self.nesting || !self.single || self.columns[0].is_listing() {
            return;
        }
        for folder in folders {
            if expanded {
                self.expanded.insert(folder.clone());
                if all {
                    self.expand_below(folder);
                }
            } else {
                self.expanded.remove(folder);
                if all {
                    self.expanded.retain(|known| !known.starts_with(folder));
                }
            }
        }
        self.nest_root();
    }

    fn expand_below(&mut self, folder: &Path) {
        let mut pending = vec![folder.to_path_buf()];
        let mut added = 0;
        while let Some(next) = pending.pop() {
            let children = list(&next, self.show_hidden, self.children_order()).unwrap_or_default();
            for child in children.into_iter().filter(Entry::is_folder) {
                if added >= MAX_EXPANDED {
                    return;
                }
                added += 1;
                self.expanded.insert(child.path.clone());
                pending.push(child.path);
            }
        }
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

    fn order(&self) -> Sort {
        Sort {
            group: self.sort.group.filter(|_| self.single),
            ..self.sort
        }
    }

    fn children_order(&self) -> Sort {
        Sort {
            group: None,
            ..self.sort
        }
    }

    pub fn groups(&self) -> Option<bande_fs::GroupBy> {
        self.order()
            .group
            .filter(|_| *self.root() != Location::Recents)
    }

    pub fn set_folders_first(&mut self, on: bool) {
        if self.sort.folders_first != on {
            self.set_sort(Sort {
                folders_first: on,
                ..self.sort
            });
        }
    }

    pub fn set_sort(&mut self, sort: Sort) {
        self.sort = sort;
        let sort = self.order();
        for column in &mut self.columns {
            if column.location != Location::Recents && !column.is_listing() && !column.is_nested() {
                column.resort(sort);
            }
        }
        self.nest_root();
        self.reload_where(|location| *location == Location::Recents);
        for index in 0..self.columns.len() {
            if self.columns.get(index).is_some_and(Column::is_listing) {
                self.reload_column(index);
            }
        }
    }

    pub fn single(&self) -> bool {
        self.single
    }

    pub fn set_single(&mut self, single: bool) {
        let regroup = self.single != single && self.sort.group.is_some();
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
        if regroup {
            self.set_sort(self.sort);
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
            let entries = self.read(&location);
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
        match shown.marked.binary_search(&row) {
            Ok(index) => {
                shown.marked.remove(index);
                shown.selected = shown.marked.last().copied();
            }
            Err(index) => {
                shown.marked.insert(index, row);
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

    pub fn mark(&mut self, column: usize, rows: Vec<usize>) {
        let Some(shown) = self.columns.get_mut(column) else {
            return;
        };
        let mut rows = rows
            .into_iter()
            .filter(|row| *row < shown.entries.len())
            .collect::<Vec<_>>();
        rows.sort_unstable();
        rows.dedup();
        let added = rows
            .iter()
            .rev()
            .find(|row| shown.marked.binary_search(row).is_err())
            .copied();
        let kept = shown.selected.filter(|row| rows.binary_search(row).is_ok());
        shown.selected = added.or(kept).or_else(|| rows.last().copied());
        shown.anchor = shown.selected;
        shown.marked = rows;
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
        let entries = self.read(&location);
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
        let mut watched = self
            .columns
            .iter()
            .flat_map(|column| match &column.location {
                Location::Folder(folder) => vec![folder.clone()],
                Location::Trash => self.trash_info_dirs(),
                Location::Recents | Location::Search(_) => Vec::new(),
            })
            .collect::<Vec<_>>();
        watched.extend(self.open_subfolders());
        watched
    }

    fn open_subfolders(&self) -> Vec<PathBuf> {
        let column = &self.columns[0];
        column
            .depths
            .iter()
            .zip(&column.entries)
            .filter(|(_, entry)| entry.is_folder() && self.expanded.contains(&entry.path))
            .map(|(_, entry)| entry.path.clone())
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
        let location = column.location.clone();
        let Contents {
            entries,
            listing,
            denied,
        } = self.read(&location);
        let (entries, depths) = match (index, &listing) {
            (0, None) => self.nested(entries),
            _ => (entries, Vec::new()),
        };
        let rows = entries
            .iter()
            .enumerate()
            .map(|(row, entry)| (&entry.path, row))
            .collect::<HashMap<_, _>>();
        let find = |path: &PathBuf| rows.get(path).copied();
        let mut marked = marked_paths.iter().filter_map(find).collect::<Vec<_>>();
        marked.sort_unstable();
        let selected = selected_path
            .as_ref()
            .and_then(find)
            .or_else(|| marked.last().copied());
        let anchor = anchor_path.as_ref().and_then(find).or(selected);
        let awaited = match listing {
            Some(_) => marked_paths
                .into_iter()
                .filter(|path| !rows.contains_key(path))
                .collect(),
            None => Vec::new(),
        };
        let lost_selection = previous_row.is_some() && selected.is_none() && awaited.is_empty();
        let child_was = self
            .columns
            .get(index + 1)
            .map(|child| child.location.clone());
        let column = &mut self.columns[index];
        if column.entries != entries {
            column.entries = entries;
            column.replaced();
        }
        column.listing = listing;
        column.denied = denied;
        column.awaited = awaited;
        column.depths = depths;
        column.marked = marked;
        column.selected = selected;
        column.anchor = anchor;
        self.follow_selection(index, previous_row, lost_selection, child_was);
    }

    fn follow_selection(
        &mut self,
        index: usize,
        previous_row: Option<usize>,
        lost_selection: bool,
        child_was: Option<Location>,
    ) {
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

    fn patch_column(&mut self, index: usize, paths: &[&PathBuf]) {
        let previous_row = self.columns[index].selected;
        let child_was = self
            .columns
            .get(index + 1)
            .map(|child| child.location.clone());
        let (sort, show_hidden) = (self.order(), self.show_hidden);
        let column = &mut self.columns[index];
        for path in paths {
            column.patch(path, sort, show_hidden);
        }
        let lost_selection = previous_row.is_some() && column.selected.is_none();
        self.follow_selection(index, previous_row, lost_selection, child_was);
    }

    pub fn apply(&mut self, changes: &FolderChanges) {
        let mut whole = changes.folders.clone();
        let open = self.open_subfolders();
        if let Some(root) = self.columns[0].folder().map(Path::to_path_buf)
            && self.columns[0].is_nested()
        {
            let touched = changes
                .entries
                .iter()
                .filter_map(|path| path.parent())
                .chain(changes.folders.iter().map(PathBuf::as_path))
                .any(|folder| folder == root || open.iter().any(|known| known == folder));
            if touched {
                whole.push(root);
            }
        }
        let info_dirs = self.trash_info_dirs();
        whole.extend(
            changes
                .entries
                .iter()
                .filter_map(|path| path.parent())
                .filter(|folder| info_dirs.iter().any(|dir| dir == folder))
                .map(Path::to_path_buf),
        );
        whole.extend(
            self.columns
                .iter()
                .filter(|column| column.is_listing())
                .filter_map(|column| column.folder())
                .filter(|folder| {
                    changes
                        .entries
                        .iter()
                        .any(|path| path.parent() == Some(*folder))
                })
                .map(Path::to_path_buf),
        );
        let mut index = 0;
        while index < self.columns.len() {
            if let Some(folder) = self.columns[index].folder()
                && !whole.iter().any(|stale| stale == folder)
            {
                let paths = changes
                    .entries
                    .iter()
                    .filter(|path| path.parent() == Some(folder))
                    .collect::<Vec<_>>();
                if !paths.is_empty() {
                    self.patch_column(index, &paths);
                }
            }
            index += 1;
        }
        self.refresh(&whole);
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
        let open = self.open_subfolders();
        let mut changed = changed.to_vec();
        if changed.iter().any(|folder| open.contains(folder))
            && let Some(root) = self.columns[0].folder().map(Path::to_path_buf)
        {
            changed.push(root);
        }
        let trash_changed = self
            .trash_info_dirs()
            .iter()
            .any(|dir| changed.contains(dir));
        self.reload_where(|location| match location {
            Location::Folder(folder) => changed.contains(folder),
            Location::Trash => trash_changed,
            Location::Recents | Location::Search(_) => true,
        });
    }

    pub fn reload(&mut self) {
        self.reload_where(|_| true);
    }

    pub fn toggle_hidden(&mut self) {
        self.set_show_hidden(!self.show_hidden);
    }

    pub fn set_show_hidden(&mut self, show: bool) {
        if show != self.show_hidden {
            self.show_hidden = show;
            self.reload();
        }
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
        let positions = column
            .entries
            .iter()
            .enumerate()
            .map(|(row, entry)| (&entry.path, row))
            .collect::<HashMap<_, _>>();
        let rows = paths
            .iter()
            .filter_map(|path| positions.get(path).copied())
            .collect::<Vec<_>>();
        if column.listing.is_some() {
            column.awaited = paths
                .iter()
                .filter(|path| !positions.contains_key(path))
                .cloned()
                .collect();
        }
        let Some(last) = rows.last().copied() else {
            if !column.awaited.is_empty() {
                column.clear();
                self.active = index;
            }
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
