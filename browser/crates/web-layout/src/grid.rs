use std::sync::Arc;

use web_style::{Align, Breadth, GridLine, GridStyle, Track, Tracks};

use crate::box_tree::BlockBox;
use crate::flow::{Bfc, Flow, Laid, Sizing};
use crate::fragment::Fragment;

const MAX_LINES: i32 = 1000;

#[derive(Clone, Copy)]
enum Edge {
    Line(i32),
    Span(i32),
    Auto,
}

struct Placed<'b> {
    block: &'b BlockBox,
    rows: (usize, usize),
    columns: (usize, usize),
}

fn expand(tracks: &Tracks, available: f32, gap: f32) -> Vec<Track> {
    let mut expanded = tracks.tracks.clone();
    if let Some((at, repeat, _)) = &tracks.repeat
        && !repeat.is_empty()
    {
        let fixed = |track: &Track| match (track.max, track.min) {
            (Breadth::Fixed(size), _) | (_, Breadth::Fixed(size)) => size.resolve(available),
            _ => 0.0,
        };
        let others: f32 = expanded.iter().map(fixed).sum::<f32>() + gap * expanded.len() as f32;
        let unit: f32 = repeat.iter().map(fixed).sum::<f32>() + gap * repeat.len() as f32;
        let count = if available.is_finite() && unit > 0.0 {
            (((available - others + gap) / unit).floor() as usize).clamp(1, 1000)
        } else {
            1
        };
        let at = (*at as usize).min(expanded.len());
        let insert: Vec<Track> = (0..count).flat_map(|_| repeat.iter().copied()).collect();
        expanded.splice(at..at, insert);
    }
    expanded
}

fn named_line(tracks: &Tracks, name: &str) -> Option<i32> {
    tracks
        .names
        .iter()
        .find(|(known, _)| &**known == name)
        .map(|(_, line)| *line as i32)
}

fn edge(line: &GridLine, grid: &GridStyle, columns: bool, end: bool, explicit: i32) -> Edge {
    let tracks = if columns { &grid.columns } else { &grid.rows };
    match line {
        GridLine::Auto => Edge::Auto,
        GridLine::Span(count, _) => Edge::Span((*count as i32).min(MAX_LINES)),
        GridLine::Line(number, name) => {
            if let Some(name) = name
                && let Some(line) = named_line(tracks, name)
            {
                return Edge::Line(line);
            }
            let number = (*number).clamp(-MAX_LINES, MAX_LINES);
            if number > 0 {
                Edge::Line(number - 1)
            } else {
                Edge::Line((explicit + 1 + number).max(0))
            }
        }
        GridLine::Name(name) => {
            let suffix = if end { "-end" } else { "-start" };
            if let Some(line) = named_line(tracks, &format!("{name}{suffix}")) {
                return Edge::Line(line);
            }
            if let Some(area) = grid.areas.iter().find(|area| area.name == *name) {
                let (start, stop) = if columns { area.columns } else { area.rows };
                return Edge::Line(if end { stop } else { start } as i32);
            }
            match named_line(tracks, name) {
                Some(line) => Edge::Line(line),
                None => Edge::Auto,
            }
        }
    }
}

fn axis(start: Edge, end: Edge) -> (Option<usize>, usize) {
    match (start, end) {
        (Edge::Line(a), Edge::Line(b)) => {
            let (low, high) = (a.min(b), a.max(b));
            (Some(low.max(0) as usize), (high - low).max(1) as usize)
        }
        (Edge::Line(a), Edge::Span(span)) => (Some(a.max(0) as usize), span.max(1) as usize),
        (Edge::Line(a), Edge::Auto) => (Some(a.max(0) as usize), 1),
        (Edge::Span(span), Edge::Line(b)) => {
            (Some((b - span).max(0) as usize), span.max(1) as usize)
        }
        (Edge::Auto, Edge::Line(b)) => (Some((b - 1).max(0) as usize), 1),
        (Edge::Span(span), _) | (_, Edge::Span(span)) => (None, span.max(1) as usize),
        (Edge::Auto, Edge::Auto) => (None, 1),
    }
}

struct Occupancy {
    cells: Vec<Vec<bool>>,
}

impl Occupancy {
    fn free(&self, row: usize, column: usize, rows: usize, columns: usize) -> bool {
        (row..row + rows).all(|r| {
            (column..column + columns).all(|c| {
                !self
                    .cells
                    .get(r)
                    .and_then(|line| line.get(c))
                    .copied()
                    .unwrap_or(false)
            })
        })
    }

    fn take(&mut self, row: usize, column: usize, rows: usize, columns: usize) {
        if self.cells.len() < row + rows {
            self.cells.resize(row + rows, Vec::new());
        }
        for line in self.cells.iter_mut().skip(row).take(rows) {
            if line.len() < column + columns {
                line.resize(column + columns, false);
            }
            for cell in line.iter_mut().skip(column).take(columns) {
                *cell = true;
            }
        }
    }
}

fn track(explicit: &[Track], implicit: &[Track], index: usize) -> Track {
    match explicit.get(index) {
        Some(track) => *track,
        None if implicit.is_empty() => Track::AUTO,
        None => implicit
            .get((index - explicit.len()) % implicit.len())
            .copied()
            .unwrap_or(Track::AUTO),
    }
}

fn size_tracks(
    tracks: &[Track],
    contributions: &[(usize, usize, f32, f32)],
    available: Option<f32>,
    gap: f32,
) -> Vec<f32> {
    let count = tracks.len();
    let basis = available.unwrap_or(0.0);
    let mut min_content = vec![0.0f32; count];
    let mut max_content = vec![0.0f32; count];
    for &(start, span, min, max) in contributions {
        if span == 1
            && let (Some(low), Some(high)) =
                (min_content.get_mut(start), max_content.get_mut(start))
        {
            *low = low.max(min);
            *high = high.max(max);
        }
    }
    let mut sizes: Vec<f32> = tracks
        .iter()
        .enumerate()
        .map(|(index, track)| match track.min {
            Breadth::Fixed(size) => size.resolve(basis),
            Breadth::MaxContent => max_content.get(index).copied().unwrap_or(0.0),
            _ => min_content.get(index).copied().unwrap_or(0.0),
        })
        .collect();
    let limits: Vec<f32> = tracks
        .iter()
        .enumerate()
        .map(|(index, track)| match track.max {
            Breadth::Fixed(size) => size.resolve(basis),
            Breadth::Flex(_) => f32::INFINITY,
            Breadth::MinContent => min_content.get(index).copied().unwrap_or(0.0),
            Breadth::Auto | Breadth::MaxContent => max_content.get(index).copied().unwrap_or(0.0),
        })
        .collect();
    for (size, limit) in sizes.iter_mut().zip(&limits) {
        if limit.is_finite() && *limit > *size && available.is_none() {
            *size = *limit;
        }
    }
    for &(start, span, min, _) in contributions {
        if span > 1 {
            let range = start..(start + span).min(count);
            let current: f32 = sizes
                .get(range.clone())
                .unwrap_or_default()
                .iter()
                .sum::<f32>()
                + gap * (span - 1) as f32;
            if min > current
                && let Some(last) = range.clone().last().and_then(|last| sizes.get_mut(last))
            {
                *last += min - current;
            }
        }
    }
    let Some(available) = available else {
        return sizes;
    };
    let gaps = gap * count.saturating_sub(1) as f32;
    let mut free = available - sizes.iter().sum::<f32>() - gaps;
    if free > 0.0 {
        let growable: Vec<usize> = (0..count)
            .filter(|&index| {
                limits.get(index).is_some_and(|limit| limit.is_finite())
                    && sizes.get(index) < limits.get(index)
            })
            .collect();
        for _ in 0..4 {
            let open: Vec<usize> = growable
                .iter()
                .copied()
                .filter(|&index| sizes.get(index) < limits.get(index))
                .collect();
            if open.is_empty() || free <= 0.0 {
                break;
            }
            let share = free / open.len() as f32;
            for index in open {
                if let (Some(size), Some(limit)) = (sizes.get_mut(index), limits.get(index)) {
                    let grown = (*size + share).min(*limit);
                    free -= grown - *size;
                    *size = grown;
                }
            }
        }
    }
    let flex: f32 = tracks
        .iter()
        .map(|track| match track.max {
            Breadth::Flex(value) => value,
            _ => 0.0,
        })
        .sum();
    if flex > 0.0 {
        let fixed: f32 = tracks
            .iter()
            .zip(&sizes)
            .filter(|(track, _)| !matches!(track.max, Breadth::Flex(_)))
            .map(|(_, size)| *size)
            .sum();
        let unit = ((available - fixed - gaps) / flex.max(1.0)).max(0.0);
        for (track, size) in tracks.iter().zip(sizes.iter_mut()) {
            if let Breadth::Flex(value) = track.max {
                *size = size.max(unit * value);
            }
        }
    } else if free > 0.0 {
        let auto: Vec<usize> = (0..count)
            .filter(|&index| {
                tracks
                    .get(index)
                    .is_some_and(|track| track.max == Breadth::Auto)
            })
            .collect();
        if !auto.is_empty() {
            let share = free / auto.len() as f32;
            for index in auto {
                if let Some(size) = sizes.get_mut(index) {
                    *size += share;
                }
            }
        }
    }
    sizes
}

fn offsets(sizes: &[f32], gap: f32) -> Vec<f32> {
    let mut position = 0.0;
    sizes
        .iter()
        .map(|size| {
            let start = position;
            position += size + gap;
            start
        })
        .collect()
}

fn span_size(sizes: &[f32], start: usize, span: usize, gap: f32) -> f32 {
    sizes
        .get(start..start + span)
        .unwrap_or_default()
        .iter()
        .sum::<f32>()
        + gap * span.saturating_sub(1) as f32
}

impl<'a, 'b> Flow<'a, 'b> {
    pub(crate) fn grid(
        &mut self,
        block: &'b BlockBox,
        items: &'b [BlockBox],
        size: (f32, Option<f32>),
        origin: (f32, f32),
        offset: (f32, f32),
        children: &mut Vec<Fragment>,
    ) -> f32 {
        let reset = &block.style.reset;
        let grid: Arc<GridStyle> = reset.grid.clone().unwrap_or_default();
        let (width, height) = size;
        let column_gap = reset.column_gap.resolve(width);
        let row_gap = reset.row_gap.resolve(height.unwrap_or(0.0));
        let column_tracks = expand(&grid.columns, width, column_gap);
        let row_tracks = expand(&grid.rows, height.unwrap_or(f32::INFINITY), row_gap);
        let explicit_columns = column_tracks.len().max(grid.area_columns as usize) as i32;
        let explicit_rows = row_tracks.len().max(grid.area_rows as usize) as i32;
        let mut occupancy = Occupancy { cells: Vec::new() };
        let mut placed: Vec<Placed<'b>> = Vec::new();
        let mut deferred = Vec::new();
        let mut ordered: Vec<&'b BlockBox> = Vec::new();
        for item in items {
            if item.style.is_out_of_flow() {
                self.register_absolute(item, origin);
            } else {
                ordered.push(item);
            }
        }
        ordered.sort_by_key(|item| item.style.reset.order);
        for item in ordered {
            let lines = item.style.reset.placement.clone();
            let line = |index: usize| {
                lines
                    .as_ref()
                    .and_then(|lines| lines.get(index).cloned())
                    .unwrap_or(GridLine::Auto)
            };
            let row = axis(
                edge(&line(0), &grid, false, false, explicit_rows),
                edge(&line(2), &grid, false, true, explicit_rows),
            );
            let column = axis(
                edge(&line(1), &grid, true, false, explicit_columns),
                edge(&line(3), &grid, true, true, explicit_columns),
            );
            match (row.0, column.0) {
                (Some(r), Some(c)) => {
                    occupancy.take(r, c, row.1, column.1);
                    placed.push(Placed {
                        block: item,
                        rows: (r, row.1),
                        columns: (c, column.1),
                    });
                }
                _ => deferred.push((item, row, column)),
            }
        }
        let column_limit = explicit_columns.max(1) as usize;
        let mut cursor = (0usize, 0usize);
        for (item, row, column) in deferred {
            let span_columns = column.1.min(MAX_LINES as usize);
            let span_rows = row.1.min(MAX_LINES as usize);
            let (r, c) = match (row.0, column.0) {
                (Some(r), None) => {
                    let width = column_limit.max(span_columns);
                    let c = (0..=width.saturating_sub(span_columns))
                        .find(|&c| occupancy.free(r, c, span_rows, span_columns))
                        .unwrap_or(width);
                    (r, c)
                }
                (None, Some(c)) => {
                    let r = (0..)
                        .take(10_000)
                        .find(|&r| occupancy.free(r, c, span_rows, span_columns))
                        .unwrap_or(0);
                    (r, c)
                }
                _ => {
                    let width = column_limit.max(span_columns);
                    let mut found = None;
                    for _ in 0..100_000 {
                        if cursor.1 + span_columns > width {
                            cursor = (cursor.0 + 1, 0);
                        }
                        if occupancy.free(cursor.0, cursor.1, span_rows, span_columns) {
                            found = Some(cursor);
                            cursor.1 += span_columns;
                            break;
                        }
                        cursor.1 += 1;
                    }
                    found.unwrap_or(cursor)
                }
            };
            occupancy.take(r, c, span_rows, span_columns);
            placed.push(Placed {
                block: item,
                rows: (r, span_rows),
                columns: (c, span_columns),
            });
        }
        let column_count = placed
            .iter()
            .map(|item| item.columns.0 + item.columns.1)
            .max()
            .unwrap_or(0)
            .max(explicit_columns as usize);
        let row_count = placed
            .iter()
            .map(|item| item.rows.0 + item.rows.1)
            .max()
            .unwrap_or(0)
            .max(explicit_rows as usize);
        let columns: Vec<Track> = (0..column_count)
            .map(|index| track(&column_tracks, &grid.auto_columns, index))
            .collect();
        let rows: Vec<Track> = (0..row_count)
            .map(|index| track(&row_tracks, &grid.auto_rows, index))
            .collect();
        let contributions: Vec<(usize, usize, f32, f32)> = placed
            .iter()
            .map(|item| {
                let (min, max) = self.outer_intrinsic(item.block, (width, height));
                (item.columns.0, item.columns.1, min, max)
            })
            .collect();
        let column_sizes = size_tracks(&columns, &contributions, Some(width), column_gap);
        let mark = self.mark();
        let mut laid: Vec<Laid> = Vec::with_capacity(placed.len());
        for item in &placed {
            let area = span_size(&column_sizes, item.columns.0, item.columns.1, column_gap);
            laid.push(self.grid_item(item.block, area, None, origin));
        }
        self.rewind(mark);
        let row_contributions: Vec<(usize, usize, f32, f32)> = placed
            .iter()
            .zip(&laid)
            .map(|(item, laid)| {
                let outer = laid.outer().1;
                (item.rows.0, item.rows.1, outer, outer)
            })
            .collect();
        let row_sizes = size_tracks(&rows, &row_contributions, height, row_gap);
        let column_offsets = offsets(&column_sizes, column_gap);
        let row_offsets = offsets(&row_sizes, row_gap);
        for (item, mut laid) in placed.iter().zip(laid) {
            let area_width = span_size(&column_sizes, item.columns.0, item.columns.1, column_gap);
            let area_height = span_size(&row_sizes, item.rows.0, item.rows.1, row_gap);
            let align = match item.block.style.reset.align_self {
                Align::Auto => reset.align_items,
                own => own,
            };
            let stretched = (matches!(align, Align::Normal | Align::Stretch)
                && item.block.style.reset.height.is_none())
            .then(|| (area_height - laid.margin.top - laid.margin.bottom).max(0.0));
            laid = self.grid_item(item.block, area_width, stretched, origin);
            let (_, outer_height) = laid.outer();
            let shift = match align {
                Align::Center => (area_height - outer_height) / 2.0,
                Align::End => area_height - outer_height,
                _ => 0.0,
            }
            .max(0.0);
            let mut fragment = laid.fragment;
            fragment.rect.x = offset.0
                + column_offsets.get(item.columns.0).copied().unwrap_or(0.0)
                + laid.margin.left;
            fragment.rect.y = offset.1
                + row_offsets.get(item.rows.0).copied().unwrap_or(0.0)
                + laid.margin.top
                + shift;
            children.push(Fragment::Box(fragment));
        }
        let total =
            row_sizes.iter().sum::<f32>() + row_gap * row_sizes.len().saturating_sub(1) as f32;
        height.unwrap_or(total)
    }

    fn grid_item(
        &mut self,
        block: &'b BlockBox,
        width: f32,
        height: Option<f32>,
        origin: (f32, f32),
    ) -> Laid {
        let margins = crate::flow::resolve_margins(&block.style, width);
        let horizontal = margins.left.unwrap_or(0.0) + margins.right.unwrap_or(0.0);
        let border_box = if block.style.reset.width.is_some() {
            None
        } else {
            Some((width - horizontal).max(0.0))
        };
        self.block(
            block,
            (width, None),
            origin,
            &mut Bfc::default(),
            Sizing {
                width: border_box,
                height,
                shrink: border_box.is_none(),
                root: false,
            },
        )
    }
}
