use web_style::{Color, VerticalAlign};

use crate::box_tree::{BlockBox, Table, TableCell};
use crate::flow::{Bfc, Flow, Sizing};
use crate::fragment::{BoxFragment, Fragment, Rect};

struct Slot<'b> {
    row: usize,
    column: usize,
    rows: usize,
    columns: usize,
    cell: &'b TableCell,
}

fn grid(table: &Table) -> (Vec<Slot<'_>>, usize) {
    let row_count = table.rows.len();
    let mut occupied: Vec<Vec<bool>> = vec![Vec::new(); row_count];
    let mut slots = Vec::new();
    let mut columns = 0;
    for (row_index, row) in table.rows.iter().enumerate() {
        let mut column = 0;
        for cell in &row.cells {
            while occupied
                .get(row_index)
                .and_then(|row| row.get(column))
                .copied()
                .unwrap_or(false)
            {
                column += 1;
            }
            let rows = usize::from(cell.rows).min(row_count - row_index).max(1);
            let span = usize::from(cell.columns).max(1);
            for taken in occupied.iter_mut().skip(row_index).take(rows) {
                if taken.len() < column + span {
                    taken.resize(column + span, false);
                }
                for slot in taken.iter_mut().skip(column).take(span) {
                    *slot = true;
                }
            }
            slots.push(Slot {
                row: row_index,
                column,
                rows,
                columns: span,
                cell,
            });
            column += span;
            columns = columns.max(column);
        }
    }
    (slots, columns)
}

fn spacing(block: &BlockBox) -> (f32, f32) {
    let inherited = &block.style.inherited;
    if inherited.border_collapse {
        (0.0, 0.0)
    } else {
        inherited.border_spacing
    }
}

impl<'a, 'b> Flow<'a, 'b> {
    fn columns(&mut self, block: &'b BlockBox, table: &'b Table, width: f32) -> Vec<(f32, f32)> {
        let (slots, count) = grid(table);
        let (horizontal, _) = spacing(block);
        let mut columns = vec![(0.0f32, 0.0f32); count];
        let mut spanning = Vec::new();
        for slot in &slots {
            let (mut min, mut max) = self.outer_intrinsic(&slot.cell.block, (width, None));
            if let Some(fixed) = slot
                .cell
                .block
                .style
                .reset
                .width
                .filter(|width| width.percent == 0.0)
            {
                let frame = crate::flow::frame(&slot.cell.block.style, width).0;
                let fixed = if slot.cell.block.style.reset.border_box {
                    fixed.px
                } else {
                    fixed.px + frame
                };
                min = min.max(fixed);
                max = max.max(fixed).min(fixed.max(min));
            }
            if slot.columns == 1 {
                if let Some(column) = columns.get_mut(slot.column) {
                    column.0 = column.0.max(min);
                    column.1 = column.1.max(max);
                }
            } else {
                spanning.push((slot.column, slot.columns, min, max));
            }
        }
        for (start, span, min, max) in spanning {
            let gaps = horizontal * (span - 1) as f32;
            let range = start..(start + span).min(count);
            let current_min: f32 = columns
                .get(range.clone())
                .unwrap_or_default()
                .iter()
                .map(|c| c.0)
                .sum();
            let current_max: f32 = columns
                .get(range.clone())
                .unwrap_or_default()
                .iter()
                .map(|c| c.1)
                .sum();
            let extra_min = (min - gaps - current_min).max(0.0);
            let extra_max = (max - gaps - current_max).max(0.0);
            let share = range.len().max(1) as f32;
            for column in columns.get_mut(range).unwrap_or_default() {
                let weight = if current_max > 0.0 {
                    column.1 / current_max
                } else {
                    1.0 / share
                };
                column.0 += extra_min * weight;
                column.1 += extra_max * weight;
                column.1 = column.1.max(column.0);
            }
        }
        columns
    }

    pub(crate) fn table_intrinsic(
        &mut self,
        block: &'b BlockBox,
        table: &'b Table,
        width: f32,
    ) -> (f32, f32) {
        let columns = self.columns(block, table, width);
        let gaps = spacing(block).0 * (columns.len() + 1) as f32;
        let min: f32 = columns.iter().map(|column| column.0).sum::<f32>() + gaps;
        let max: f32 = columns.iter().map(|column| column.1).sum::<f32>() + gaps;
        let captions = table
            .captions
            .iter()
            .map(|caption| self.outer_intrinsic(caption, (width, None)).0)
            .fold(0.0, f32::max);
        (min.max(captions), max.max(captions))
    }

    pub(crate) fn table(
        &mut self,
        block: &'b BlockBox,
        table: &'b Table,
        width: f32,
        origin: (f32, f32),
        offset: (f32, f32),
        children: &mut Vec<Fragment>,
    ) -> f32 {
        let mut y = 0.0;
        for caption in &table.captions {
            let laid = self.block(
                caption,
                (width, None),
                origin,
                &mut Bfc::default(),
                Sizing::default(),
            );
            let mut fragment = laid.fragment;
            fragment.rect.x += offset.0;
            fragment.rect.y = offset.1 + y + laid.margin.top;
            y += laid.margin.top + fragment.rect.height + laid.margin.bottom;
            children.push(Fragment::Box(fragment));
        }
        let (horizontal, vertical) = spacing(block);
        let mut columns = self.columns(block, table, width);
        let inner = (width - horizontal * (columns.len() + 1) as f32).max(0.0);
        let min: f32 = columns.iter().map(|column| column.0).sum();
        let max: f32 = columns.iter().map(|column| column.1).sum();
        let widths: Vec<f32> = if inner >= max {
            let extra = inner - max;
            let count = columns.len().max(1) as f32;
            columns
                .iter()
                .map(|column| {
                    column.1
                        + if max > 0.0 {
                            extra * column.1 / max
                        } else {
                            extra / count
                        }
                })
                .collect()
        } else if inner > min && max > min {
            let ratio = (inner - min) / (max - min);
            columns
                .iter()
                .map(|column| column.0 + (column.1 - column.0) * ratio)
                .collect()
        } else {
            columns.drain(..).map(|column| column.0).collect()
        };
        let mut lefts = Vec::with_capacity(widths.len());
        let mut x = horizontal;
        for width in &widths {
            lefts.push(x);
            x += width + horizontal;
        }
        let (slots, _) = grid(table);
        let span_width = |slot: &Slot<'_>| {
            widths
                .get(slot.column..slot.column + slot.columns)
                .unwrap_or_default()
                .iter()
                .sum::<f32>()
                + horizontal * (slot.columns - 1) as f32
        };
        let mut heights = vec![0.0f32; table.rows.len()];
        for (row, height) in table.rows.iter().zip(heights.iter_mut()) {
            if let Some(specified) = row
                .style
                .reset
                .height
                .filter(|height| height.percent == 0.0)
            {
                *height = specified.px;
            }
        }
        let mut measured = Vec::with_capacity(slots.len());
        let mark = self.mark();
        for slot in &slots {
            let cell_width = span_width(slot);
            let laid = self.block(
                &slot.cell.block,
                (cell_width, None),
                origin,
                &mut Bfc::default(),
                Sizing {
                    width: Some(cell_width),
                    ..Sizing::default()
                },
            );
            let height = laid.fragment.rect.height;
            if slot.rows == 1
                && let Some(row) = heights.get_mut(slot.row)
            {
                *row = row.max(height);
            }
            measured.push(height);
        }
        self.rewind(mark);
        for (slot, &height) in slots.iter().zip(&measured) {
            if slot.rows > 1 {
                let range = slot.row..slot.row + slot.rows;
                let current: f32 = heights
                    .get(range.clone())
                    .unwrap_or_default()
                    .iter()
                    .sum::<f32>()
                    + vertical * (slot.rows - 1) as f32;
                if height > current
                    && let Some(last) = heights.get_mut(range.end - 1)
                {
                    *last += height - current;
                }
            }
        }
        let mut tops = Vec::with_capacity(heights.len());
        let mut top = vertical;
        for height in &heights {
            tops.push(top);
            top += height + vertical;
        }
        let grid_height = if heights.is_empty() { 0.0 } else { top };
        for (row, (&row_top, &row_height)) in table.rows.iter().zip(tops.iter().zip(&heights)) {
            let background = row.style.reset.background_color;
            if background != Color::TRANSPARENT && row.node.is_some() {
                children.push(Fragment::Box(BoxFragment {
                    node: row.node,
                    rect: Rect {
                        x: offset.0 + horizontal,
                        y: offset.1 + y + row_top,
                        width: (x - 2.0 * horizontal).max(0.0),
                        height: row_height,
                    },
                    background,
                    extra: None,
                    children: Vec::new(),
                }));
            }
        }
        for (slot, &content) in slots.iter().zip(&measured) {
            let cell_width = span_width(slot);
            let cell_height = heights
                .get(slot.row..slot.row + slot.rows)
                .unwrap_or_default()
                .iter()
                .sum::<f32>()
                + vertical * (slot.rows - 1) as f32;
            let laid = self.block(
                &slot.cell.block,
                (cell_width, None),
                origin,
                &mut Bfc::default(),
                Sizing {
                    width: Some(cell_width),
                    height: Some(cell_height),
                    ..Sizing::default()
                },
            );
            let shift = match slot.cell.block.style.reset.vertical_align {
                VerticalAlign::Middle => (cell_height - content) / 2.0,
                VerticalAlign::Bottom => cell_height - content,
                _ => 0.0,
            }
            .max(0.0);
            let mut fragment = laid.fragment;
            if shift > 0.0 {
                for child in &mut fragment.children {
                    match child {
                        Fragment::Box(child) => child.rect.y += shift,
                        Fragment::Text(text) => {
                            text.rect.y += shift;
                            text.baseline += shift;
                        }
                    }
                }
            }
            fragment.rect.x = offset.0 + lefts.get(slot.column).copied().unwrap_or(0.0);
            fragment.rect.y = offset.1 + y + tops.get(slot.row).copied().unwrap_or(0.0);
            children.push(Fragment::Box(fragment));
        }
        y + grid_height
    }
}
