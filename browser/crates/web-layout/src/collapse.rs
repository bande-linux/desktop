use std::sync::Arc;

use web_style::{BorderStyle, Color, ComputedStyle, ResetStyle, Sides};

use crate::box_tree::{Parts, Table};
use crate::table::grid;

#[derive(Clone, Copy)]
pub(crate) struct Edge {
    pub(crate) width: f32,
    pub(crate) color: Color,
    style: BorderStyle,
    origin: u8,
}

pub(crate) struct Collapsed {
    pub(crate) columns: usize,
    pub(crate) horizontal: Vec<Edge>,
    pub(crate) vertical: Vec<Edge>,
}

impl Collapsed {
    pub(crate) fn horizontal(&self, row: usize, column: usize) -> Option<Edge> {
        self.horizontal.get(row * self.columns + column).copied()
    }

    pub(crate) fn vertical(&self, row: usize, column: usize) -> Option<Edge> {
        self.vertical
            .get(row * (self.columns + 1) + column)
            .copied()
    }
}

const CELL: u8 = 0;
const ROW: u8 = 1;
const GROUP: u8 = 2;
const COLUMN: u8 = 3;
const COLUMN_GROUP: u8 = 4;
const TABLE: u8 = 5;

fn side(style: &ComputedStyle, index: usize, origin: u8) -> Edge {
    let reset = &style.reset;
    let (width, color, style) = match index {
        0 => (
            reset.border.top,
            reset.border_color.top,
            reset.border_style.top,
        ),
        1 => (
            reset.border.right,
            reset.border_color.right,
            reset.border_style.right,
        ),
        2 => (
            reset.border.bottom,
            reset.border_color.bottom,
            reset.border_style.bottom,
        ),
        _ => (
            reset.border.left,
            reset.border_color.left,
            reset.border_style.left,
        ),
    };
    Edge {
        width,
        color,
        style,
        origin,
    }
}

fn rank(style: BorderStyle) -> u8 {
    match style {
        BorderStyle::Double => 8,
        BorderStyle::Solid => 7,
        BorderStyle::Dashed => 6,
        BorderStyle::Dotted => 5,
        BorderStyle::Ridge => 4,
        BorderStyle::Outset => 3,
        BorderStyle::Groove => 2,
        BorderStyle::Inset => 1,
        BorderStyle::None | BorderStyle::Hidden => 0,
    }
}

fn beats(new: &Edge, current: &Edge) -> bool {
    if current.style == BorderStyle::Hidden {
        return false;
    }
    if new.style == BorderStyle::Hidden {
        return true;
    }
    if new.style == BorderStyle::None {
        return false;
    }
    if current.style == BorderStyle::None || new.width > current.width {
        return true;
    }
    if new.width < current.width {
        return false;
    }
    match rank(new.style).cmp(&rank(current.style)) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => new.origin < current.origin,
    }
}

fn resolve(candidates: impl IntoIterator<Item = Edge>) -> Edge {
    let mut winner = Edge {
        width: 0.0,
        color: Color::TRANSPARENT,
        style: BorderStyle::None,
        origin: TABLE + 1,
    };
    for candidate in candidates {
        if beats(&candidate, &winner) {
            winner = candidate;
        }
    }
    if matches!(winner.style, BorderStyle::None | BorderStyle::Hidden) {
        winner.width = 0.0;
    }
    winner
}

pub(crate) fn collapse(table: &mut Table, style: &ComputedStyle, parts: &Parts) -> Sides {
    let (slots, columns) = grid(table);
    let columns = columns.max(parts.columns.len());
    let rows = table.rows.len();
    let mut owner = vec![None; rows * columns];
    for (index, slot) in slots.iter().enumerate() {
        for row in slot.row..slot.row + slot.rows {
            for column in slot.column..slot.column + slot.columns {
                if let Some(cell) = owner.get_mut(row * columns + column) {
                    *cell = Some(index);
                }
            }
        }
    }
    let owner_at = |row: usize, column: usize| {
        (row < rows && column < columns)
            .then(|| owner.get(row * columns + column).copied().flatten())
            .flatten()
    };
    let group_of = |row: usize| {
        let id = table.rows.get(row).and_then(|row| row.group)?;
        parts.groups.get(id as usize).map(|style| (id, style))
    };
    let column_of = |column: usize| parts.columns.get(column);
    let column_group_of = |column: usize| {
        let (_, group) = column_of(column)?;
        let group = (*group)?;
        parts.column_groups.get(group).map(|style| (group, style))
    };
    let mut horizontal = Vec::with_capacity((rows + 1) * columns);
    for row in 0..=rows {
        let above_group = row.checked_sub(1).and_then(group_of);
        let below_group = group_of(row);
        let boundary = above_group.map(|group| group.0) != below_group.map(|group| group.0);
        for column in 0..columns {
            let above = row.checked_sub(1).and_then(|row| owner_at(row, column));
            let below = owner_at(row, column);
            if above.is_some() && above == below {
                horizontal.push(resolve([]));
                continue;
            }
            let mut candidates = Vec::with_capacity(8);
            candidates.extend(
                above
                    .and_then(|index| slots.get(index))
                    .map(|slot| side(&slot.cell.block.style, 2, CELL)),
            );
            candidates.extend(
                below
                    .and_then(|index| slots.get(index))
                    .map(|slot| side(&slot.cell.block.style, 0, CELL)),
            );
            candidates.extend(
                row.checked_sub(1)
                    .and_then(|row| table.rows.get(row))
                    .map(|row| side(&row.style, 2, ROW)),
            );
            candidates.extend(table.rows.get(row).map(|row| side(&row.style, 0, ROW)));
            if boundary {
                candidates.extend(above_group.map(|group| side(group.1, 2, GROUP)));
                candidates.extend(below_group.map(|group| side(group.1, 0, GROUP)));
            }
            if row == 0 {
                candidates.extend(column_of(column).map(|(style, _)| side(style, 0, COLUMN)));
                candidates
                    .extend(column_group_of(column).map(|group| side(group.1, 0, COLUMN_GROUP)));
                candidates.push(side(style, 0, TABLE));
            }
            if row == rows {
                candidates.extend(column_of(column).map(|(style, _)| side(style, 2, COLUMN)));
                candidates
                    .extend(column_group_of(column).map(|group| side(group.1, 2, COLUMN_GROUP)));
                candidates.push(side(style, 2, TABLE));
            }
            horizontal.push(resolve(candidates));
        }
    }
    let mut vertical = Vec::with_capacity(rows * (columns + 1));
    for row in 0..rows {
        let group = group_of(row);
        let style_row = table.rows.get(row);
        for column in 0..=columns {
            let left = column
                .checked_sub(1)
                .and_then(|column| owner_at(row, column));
            let right = owner_at(row, column);
            if left.is_some() && left == right {
                vertical.push(resolve([]));
                continue;
            }
            let mut candidates = Vec::with_capacity(8);
            candidates.extend(
                left.and_then(|index| slots.get(index))
                    .map(|slot| side(&slot.cell.block.style, 1, CELL)),
            );
            candidates.extend(
                right
                    .and_then(|index| slots.get(index))
                    .map(|slot| side(&slot.cell.block.style, 3, CELL)),
            );
            let before = column.checked_sub(1);
            candidates.extend(
                before
                    .and_then(column_of)
                    .map(|(style, _)| side(style, 1, COLUMN)),
            );
            candidates.extend(column_of(column).map(|(style, _)| side(style, 3, COLUMN)));
            let before_group = before.and_then(column_group_of);
            let after_group = column_group_of(column);
            if before_group.map(|group| group.0) != after_group.map(|group| group.0) {
                candidates.extend(before_group.map(|group| side(group.1, 1, COLUMN_GROUP)));
                candidates.extend(after_group.map(|group| side(group.1, 3, COLUMN_GROUP)));
            }
            if column == 0 {
                candidates.extend(style_row.map(|row| side(&row.style, 3, ROW)));
                candidates.extend(group.map(|group| side(group.1, 3, GROUP)));
                candidates.push(side(style, 3, TABLE));
            }
            if column == columns {
                candidates.extend(style_row.map(|row| side(&row.style, 1, ROW)));
                candidates.extend(group.map(|group| side(group.1, 1, GROUP)));
                candidates.push(side(style, 1, TABLE));
            }
            vertical.push(resolve(candidates));
        }
    }
    let collapsed = Collapsed {
        columns,
        horizontal,
        vertical,
    };
    let widest = |edges: &mut dyn Iterator<Item = Option<Edge>>| {
        edges
            .flatten()
            .map(|edge| edge.width)
            .fold(0.0f32, f32::max)
            / 2.0
    };
    let halves: Vec<Sides> = slots
        .iter()
        .map(|slot| {
            let across = slot.column..slot.column + slot.columns;
            let down = slot.row..slot.row + slot.rows;
            Sides {
                top: widest(
                    &mut across
                        .clone()
                        .map(|column| collapsed.horizontal(slot.row, column)),
                ),
                bottom: widest(
                    &mut across.map(|column| collapsed.horizontal(slot.row + slot.rows, column)),
                ),
                left: widest(&mut down.clone().map(|row| collapsed.vertical(row, slot.column))),
                right: widest(
                    &mut down.map(|row| collapsed.vertical(row, slot.column + slot.columns)),
                ),
            }
        })
        .collect();
    let outer = Sides {
        top: widest(&mut (0..columns).map(|column| collapsed.horizontal(0, column))),
        bottom: widest(&mut (0..columns).map(|column| collapsed.horizontal(rows, column))),
        left: widest(&mut std::iter::once(collapsed.vertical(0, 0))),
        right: widest(&mut std::iter::once(collapsed.vertical(0, columns))),
    };
    drop(slots);
    let mut shared: Vec<(*const ResetStyle, Sides, Arc<ResetStyle>)> = Vec::new();
    let cells = table.rows.iter_mut().flat_map(|row| row.cells.iter_mut());
    for (cell, half) in cells.zip(halves) {
        let reset = &mut cell.block.style.reset;
        let key = Arc::as_ptr(reset);
        if let Some((_, _, found)) = shared
            .iter()
            .find(|(original, sides, _)| *original == key && *sides == half)
        {
            *reset = found.clone();
            continue;
        }
        let modified = Arc::make_mut(reset);
        modified.border = half;
        modified.border_color = Sides::all(Color::TRANSPARENT);
        shared.push((key, half, reset.clone()));
    }
    table.collapsed = Some(Box::new(collapsed));
    outer
}
