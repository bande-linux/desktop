use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bande_fs::finder_order;

pub const CELL_WIDTH: f32 = 104.0;
pub const CELL_HEIGHT: f32 = 112.0;
pub const GRID_PADDING: f32 = 12.0;
pub const ICON_SIZE: f32 = 64.0;
pub const ICON_TOP: f32 = 6.0;
pub const LABEL_TOP: f32 = ICON_TOP + ICON_SIZE + 4.0;
pub const LABEL_WIDTH: f32 = 96.0;
pub const LABEL_HEIGHT: f32 = 32.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Folder,
    File,
    Volume,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub path: PathBuf,
    pub name: String,
    pub kind: Kind,
    pub kind_name: String,
    pub size: u64,
    pub added: Option<SystemTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Slot {
    pub column: u32,
    pub row: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Bounds {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn inset(self, amount: f32) -> Self {
        Self::new(
            self.x + amount,
            self.y + amount,
            (self.width - 2.0 * amount).max(0.0),
            (self.height - 2.0 * amount).max(0.0),
        )
    }

    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }

    pub fn intersects(self, other: Self) -> bool {
        self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }

    pub fn spanning(from: (f32, f32), to: (f32, f32)) -> Self {
        Self::new(
            from.0.min(to.0),
            from.1.min(to.1),
            (from.0 - to.0).abs(),
            (from.1 - to.1).abs(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub area: Bounds,
    pub columns: u32,
    pub rows: u32,
}

impl Grid {
    pub fn new(area: Bounds) -> Self {
        let fit =
            |length: f32, cell: f32| ((length - 2.0 * GRID_PADDING) / cell).floor().max(1.0) as u32;
        Self {
            area,
            columns: fit(area.width, CELL_WIDTH),
            rows: fit(area.height, CELL_HEIGHT),
        }
    }

    pub fn key(&self) -> String {
        format!("{}x{}", self.columns, self.rows)
    }

    pub fn cell(&self, slot: Slot) -> Bounds {
        Bounds::new(
            self.area.x + self.area.width - GRID_PADDING - (slot.column as f32 + 1.0) * CELL_WIDTH,
            self.area.y + GRID_PADDING + slot.row as f32 * CELL_HEIGHT,
            CELL_WIDTH,
            CELL_HEIGHT,
        )
    }

    pub fn slot_at(&self, x: f32, y: f32) -> Slot {
        let column = (self.area.x + self.area.width - GRID_PADDING - x) / CELL_WIDTH;
        let row = (y - self.area.y - GRID_PADDING) / CELL_HEIGHT;
        let clamp = |value: f32, count: u32| (value.floor().max(0.0) as u32).min(count - 1);
        Slot {
            column: clamp(column, self.columns),
            row: clamp(row, self.rows),
        }
    }

    pub fn contains(&self, slot: Slot) -> bool {
        slot.column < self.columns && slot.row < self.rows
    }

    pub fn slots(&self) -> impl Iterator<Item = Slot> + '_ {
        (0..self.columns).flat_map(|column| (0..self.rows).map(move |row| Slot { column, row }))
    }
}

pub fn icon_bounds(cell: Bounds) -> Bounds {
    Bounds::new(
        cell.x + (cell.width - ICON_SIZE) / 2.0,
        cell.y + ICON_TOP,
        ICON_SIZE,
        ICON_SIZE,
    )
}

pub fn label_bounds(cell: Bounds) -> Bounds {
    Bounds::new(
        cell.x + (cell.width - LABEL_WIDTH) / 2.0,
        cell.y + LABEL_TOP,
        LABEL_WIDTH,
        LABEL_HEIGHT,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortBy {
    #[default]
    None,
    Name,
    Kind,
    DateAdded,
    Size,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

pub type Positions = HashMap<String, Slot>;

#[derive(Debug, Clone, PartialEq)]
pub struct Desktop {
    items: Vec<Item>,
    slots: Vec<Slot>,
    grid: Grid,
    sort: SortBy,
    selected: BTreeSet<PathBuf>,
    cursor: Option<PathBuf>,
}

fn compare(by: SortBy, a: &Item, b: &Item) -> Ordering {
    let volumes_first = (b.kind == Kind::Volume).cmp(&(a.kind == Kind::Volume));
    let order = match by {
        SortBy::None | SortBy::Name => Ordering::Equal,
        SortBy::Kind => (b.kind == Kind::Folder)
            .cmp(&(a.kind == Kind::Folder))
            .then_with(|| finder_order(&a.kind_name, &b.kind_name)),
        SortBy::DateAdded => b.added.cmp(&a.added),
        SortBy::Size => b.size.cmp(&a.size),
    };
    volumes_first
        .then(order)
        .then_with(|| finder_order(&a.name, &b.name))
}

impl Desktop {
    pub fn new(grid: Grid) -> Self {
        Self {
            items: Vec::new(),
            slots: Vec::new(),
            grid,
            sort: SortBy::None,
            selected: BTreeSet::new(),
            cursor: None,
        }
    }

    pub fn items(&self) -> impl Iterator<Item = (&Item, Slot)> {
        self.items.iter().zip(self.slots.iter().copied())
    }

    pub fn item(&self, path: &Path) -> Option<&Item> {
        self.items.iter().find(|item| item.path == path)
    }

    pub fn slot_of(&self, path: &Path) -> Option<Slot> {
        self.index(path).map(|index| self.slots[index])
    }

    fn index(&self, path: &Path) -> Option<usize> {
        self.items.iter().position(|item| item.path == path)
    }

    pub fn grid(&self) -> Grid {
        self.grid
    }

    pub fn sort(&self) -> SortBy {
        self.sort
    }

    pub fn positions(&self) -> Positions {
        self.items
            .iter()
            .zip(&self.slots)
            .filter(|(item, _)| item.kind != Kind::Volume)
            .map(|(item, slot)| (item.name.clone(), *slot))
            .collect()
    }

    fn first_free(&self, taken: &BTreeSet<Slot>) -> Slot {
        self.grid
            .slots()
            .find(|slot| !taken.contains(slot))
            .unwrap_or(Slot {
                column: self.grid.columns - 1,
                row: self.grid.rows - 1,
            })
    }

    pub fn set_items(&mut self, items: Vec<Item>, remembered: &Positions) {
        let previous: HashMap<PathBuf, Slot> = self
            .items
            .drain(..)
            .map(|item| item.path)
            .zip(self.slots.drain(..))
            .collect();
        let mut taken = BTreeSet::new();
        let mut placed: Vec<Option<Slot>> = Vec::with_capacity(items.len());
        for item in &items {
            let slot = previous
                .get(&item.path)
                .or_else(|| {
                    remembered
                        .get(&item.name)
                        .filter(|_| item.kind != Kind::Volume)
                })
                .copied()
                .filter(|slot| self.grid.contains(*slot) && !taken.contains(slot));
            if let Some(slot) = slot {
                taken.insert(slot);
            }
            placed.push(slot);
        }
        self.items = items;
        self.slots = placed
            .into_iter()
            .map(|slot| {
                slot.unwrap_or_else(|| {
                    let free = self.first_free(&taken);
                    taken.insert(free);
                    free
                })
            })
            .collect();
        let paths: BTreeSet<&PathBuf> = self.items.iter().map(|item| &item.path).collect();
        self.selected.retain(|path| paths.contains(path));
        if self
            .cursor
            .as_ref()
            .is_some_and(|cursor| !paths.contains(cursor))
        {
            self.cursor = None;
        }
        if self.sort != SortBy::None {
            self.arrange(self.sort);
        }
    }

    pub fn set_grid(&mut self, grid: Grid, remembered: &Positions) {
        if grid == self.grid {
            return;
        }
        let same_shape = grid.columns == self.grid.columns && grid.rows == self.grid.rows;
        self.grid = grid;
        if same_shape {
            return;
        }
        let items = std::mem::take(&mut self.items);
        self.slots.clear();
        self.set_items(items, remembered);
    }

    fn grid_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.items.len()).collect();
        order.sort_by_key(|index| self.slots[*index]);
        order
    }

    pub fn clean_up(&mut self) {
        let order = self.grid_order();
        self.pack(&order);
    }

    pub fn arrange(&mut self, by: SortBy) {
        self.sort = by;
        if by == SortBy::None {
            return;
        }
        let mut order: Vec<usize> = (0..self.items.len()).collect();
        order.sort_by(|a, b| compare(by, &self.items[*a], &self.items[*b]));
        self.pack(&order);
    }

    fn pack(&mut self, order: &[usize]) {
        let mut slots = self.grid.slots();
        let last = Slot {
            column: self.grid.columns - 1,
            row: self.grid.rows - 1,
        };
        for index in order {
            self.slots[*index] = slots.next().unwrap_or(last);
        }
    }

    pub fn move_items(&mut self, paths: &[PathBuf], by: (i64, i64)) {
        let moving: Vec<usize> = paths.iter().filter_map(|path| self.index(path)).collect();
        if moving.is_empty() || by == (0, 0) {
            return;
        }
        self.sort = SortBy::None;
        let mut taken: BTreeSet<Slot> = (0..self.items.len())
            .filter(|index| !moving.contains(index))
            .map(|index| self.slots[index])
            .collect();
        let shifted = |slot: Slot| {
            let clamp = |value: i64, count: u32| value.clamp(0, i64::from(count) - 1) as u32;
            Slot {
                column: clamp(i64::from(slot.column) + by.0, self.grid.columns),
                row: clamp(i64::from(slot.row) + by.1, self.grid.rows),
            }
        };
        let mut order = moving;
        order.sort_by_key(|index| self.slots[*index]);
        for index in order {
            let wanted = shifted(self.slots[index]);
            let slot = if taken.contains(&wanted) {
                self.nearest_free(wanted, &taken)
            } else {
                wanted
            };
            taken.insert(slot);
            self.slots[index] = slot;
        }
    }

    pub fn place_at(&mut self, paths: &[PathBuf], at: Slot) {
        let placing: Vec<usize> = paths.iter().filter_map(|path| self.index(path)).collect();
        if placing.is_empty() {
            return;
        }
        self.sort = SortBy::None;
        let mut taken: BTreeSet<Slot> = (0..self.items.len())
            .filter(|index| !placing.contains(index))
            .map(|index| self.slots[index])
            .collect();
        for index in placing {
            let slot = match taken.contains(&at) {
                true => self.nearest_free(at, &taken),
                false => at,
            };
            taken.insert(slot);
            self.slots[index] = slot;
        }
    }

    fn nearest_free(&self, from: Slot, taken: &BTreeSet<Slot>) -> Slot {
        self.grid
            .slots()
            .filter(|slot| !taken.contains(slot))
            .min_by_key(|slot| {
                let column = i64::from(slot.column) - i64::from(from.column);
                let row = i64::from(slot.row) - i64::from(from.row);
                (column * column + row * row, *slot)
            })
            .unwrap_or(from)
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<&Item> {
        self.items.iter().zip(&self.slots).find_map(|(item, slot)| {
            let cell = self.grid.cell(*slot);
            (icon_bounds(cell).contains(x, y) || label_bounds(cell).contains(x, y)).then_some(item)
        })
    }

    pub fn selected(&self) -> Vec<PathBuf> {
        self.grid_order()
            .into_iter()
            .map(|index| &self.items[index].path)
            .filter(|path| self.selected.contains(*path))
            .cloned()
            .collect()
    }

    pub fn is_selected(&self, path: &Path) -> bool {
        self.selected.contains(path)
    }

    pub fn cursor(&self) -> Option<&Path> {
        self.cursor.as_deref()
    }

    pub fn select_only(&mut self, path: &Path) {
        self.selected.clear();
        self.selected.insert(path.to_path_buf());
        self.cursor = Some(path.to_path_buf());
    }

    pub fn toggle(&mut self, path: &Path) {
        if !self.selected.remove(path) {
            self.selected.insert(path.to_path_buf());
        }
        self.cursor = Some(path.to_path_buf());
    }

    pub fn extend(&mut self, path: &Path) {
        self.selected.insert(path.to_path_buf());
        self.cursor = Some(path.to_path_buf());
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    pub fn select_all(&mut self) {
        self.selected = self.items.iter().map(|item| item.path.clone()).collect();
    }

    pub fn select_in(&mut self, area: Bounds, keep: &BTreeSet<PathBuf>) {
        let touched: BTreeSet<PathBuf> = self
            .items
            .iter()
            .zip(&self.slots)
            .filter(|(_, slot)| {
                let cell = self.grid.cell(**slot);
                icon_bounds(cell).intersects(area) || label_bounds(cell).intersects(area)
            })
            .map(|(item, _)| item.path.clone())
            .collect();
        self.selected = keep.symmetric_difference(&touched).cloned().collect();
    }

    pub fn navigate(&mut self, direction: Direction, extend: bool) {
        let current = self
            .cursor
            .as_ref()
            .and_then(|cursor| self.index(cursor))
            .or_else(|| self.grid_order().first().copied());
        let Some(current) = current else {
            return;
        };
        let target = if self.cursor.is_none() || self.selected.is_empty() {
            Some(current)
        } else {
            let from = self.slots[current];
            (0..self.items.len())
                .filter(|index| *index != current)
                .filter_map(|index| {
                    let to = self.slots[index];
                    let columns = i64::from(from.column) - i64::from(to.column);
                    let rows = i64::from(to.row) - i64::from(from.row);
                    let (along, across) = match direction {
                        Direction::Left => (-columns, rows),
                        Direction::Right => (columns, rows),
                        Direction::Up => (-rows, columns),
                        Direction::Down => (rows, columns),
                    };
                    (along > 0).then_some((along + 2 * across.abs(), along, index))
                })
                .min()
                .map(|(_, _, index)| index)
        };
        let Some(target) = target else {
            return;
        };
        let path = self.items[target].path.clone();
        if extend {
            self.extend(&path);
        } else {
            self.select_only(&path);
        }
    }

    pub fn select_prefix(&mut self, prefix: &str) -> bool {
        let prefix = prefix.to_lowercase();
        let found = self
            .items
            .iter()
            .filter(|item| item.name.to_lowercase() >= prefix)
            .min_by(|a, b| finder_order(&a.name, &b.name))
            .map(|item| item.path.clone());
        match found {
            Some(path) => {
                self.select_only(&path);
                true
            }
            None => false,
        }
    }
}
