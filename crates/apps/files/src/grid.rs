use std::collections::{HashMap, HashSet};

use bande_fs::Entry;
use bande_thumbnails::ThumbnailSize;
use bande_toolkit::ui::{Key, Modifiers, Pointer, Scroll, ScrollPhase, ScrollSource, ScrollState};
use bande_toolkit::{
    Color, Image, Item, Paragraph, Point, Radius, Rect, Shape, TextAlign, TextStyle,
};

use crate::L10N;
use crate::browser::Location;
use crate::controls::Press;
use crate::view::{FilesView, Hit};

const PADDING: f32 = 14.0;
const ICON_PAD: f32 = 6.0;
const LABEL_SIZE: f32 = 12.0;
const LABEL_LINE: f32 = 16.0;
const LABEL_GAP: f32 = 4.0;
const LABEL_INSET: f32 = 4.0;
const SELECTION_RADIUS: f32 = Radius::S;
const GROUP_HEADING: f32 = 34.0;

#[derive(Debug, Default)]
pub(crate) struct GridScroll {
    location: Option<Location>,
    vertical: ScrollState,
    rows: HashMap<String, ScrollState>,
    expanded: HashSet<String>,
    gesture: Option<String>,
}

impl GridScroll {
    pub(crate) fn shows_all(&self, group: &str) -> bool {
        self.expanded.contains(group)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GroupLayout {
    pub start: usize,
    pub expanded: bool,
    pub shift: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Section {
    start: usize,
    len: usize,
    top: f32,
    row: bool,
    shift: f32,
}

impl Section {
    fn rows(&self, columns: usize) -> usize {
        match self.row {
            true => 1,
            false => self.len.div_ceil(columns),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Grid {
    pub columns: usize,
    pub cell_width: f32,
    pub cell_height: f32,
    pub icon: f32,
    usable: f32,
    count: usize,
    grouped: bool,
    sections: Vec<Section>,
}

impl Grid {
    pub(crate) fn new(width: f32, icon: f32, count: usize, groups: &[GroupLayout]) -> Self {
        let minimum = (icon + 36.0).max(96.0);
        let usable = (width - 2.0 * PADDING).max(minimum);
        let columns = ((usable / minimum).floor() as usize).max(1);
        let cell_height = (2.0 * ICON_PAD + icon + LABEL_GAP + 2.0 * LABEL_LINE + 6.0).ceil();
        let mut sections = Vec::with_capacity(groups.len().max(1));
        let mut top = PADDING;
        for (index, group) in groups.iter().enumerate() {
            let end = groups.get(index + 1).map_or(count, |next| next.start);
            let len = end.saturating_sub(group.start);
            let section = Section {
                start: group.start,
                len,
                top: top + GROUP_HEADING,
                row: !group.expanded && len > columns,
                shift: group.shift,
            };
            top = section.top + section.rows(columns) as f32 * cell_height;
            sections.push(section);
        }
        let grouped = !sections.is_empty();
        if !grouped {
            sections.push(Section {
                start: 0,
                len: count,
                top: PADDING,
                row: false,
                shift: 0.0,
            });
        }
        Self {
            columns,
            cell_width: (usable / columns as f32).floor(),
            cell_height,
            icon,
            usable,
            count,
            grouped,
            sections,
        }
    }

    fn section(&self, index: usize) -> usize {
        self.sections
            .partition_point(|section| section.start <= index)
            .saturating_sub(1)
    }

    pub(crate) fn row_width(&self, section: usize) -> f32 {
        self.sections
            .get(section)
            .map_or(0.0, |section| section.len as f32 * self.cell_width)
    }

    pub(crate) fn viewport_width(&self) -> f32 {
        self.columns as f32 * self.cell_width
    }

    pub(crate) fn overflows(&self, section: usize) -> bool {
        self.grouped
            && self
                .sections
                .get(section)
                .is_some_and(|section| section.len > self.columns)
    }

    pub(crate) fn headings(&self) -> impl Iterator<Item = (usize, f32)> + '_ {
        self.sections
            .iter()
            .enumerate()
            .filter(|_| self.grouped)
            .map(|(index, section)| (index, section.top - GROUP_HEADING))
    }

    pub(crate) fn row_section(&self, y: f32) -> Option<usize> {
        self.sections.iter().position(|section| {
            section.row && (section.top..section.top + self.cell_height).contains(&y)
        })
    }

    pub(crate) fn content_height(&self) -> f32 {
        self.sections.last().map_or(2.0 * PADDING, |section| {
            section.top + section.rows(self.columns) as f32 * self.cell_height + PADDING
        })
    }

    pub(crate) fn cell(&self, index: usize) -> Rect {
        let section = self.sections[self.section(index)];
        let local = index - section.start;
        let (row, column, shift) = match section.row {
            true => (0, local, section.shift),
            false => (local / self.columns, local % self.columns, 0.0),
        };
        Rect::new(
            PADDING + column as f32 * self.cell_width - shift,
            section.top + row as f32 * self.cell_height,
            self.cell_width,
            self.cell_height,
        )
    }

    pub(crate) fn row_span(&self, index: usize) -> (f32, f32) {
        let section = self.sections[self.section(index)];
        let left = (index - section.start) as f32 * self.cell_width;
        (left, left + self.cell_width)
    }

    pub(crate) fn is_row(&self, section: usize) -> bool {
        self.sections
            .get(section)
            .is_some_and(|section| section.row)
    }

    pub(crate) fn in_row(&self, index: usize) -> Option<usize> {
        let section = self.section(index);
        self.is_row(section).then_some(section)
    }

    pub(crate) fn icon_rect(&self, index: usize) -> Rect {
        let cell = self.cell(index);
        Rect::new(
            cell.x + ((cell.width - self.icon) / 2.0).round(),
            cell.y + ICON_PAD,
            self.icon,
            self.icon,
        )
    }

    pub(crate) fn label_area(&self, index: usize) -> Rect {
        let cell = self.cell(index);
        Rect::new(
            cell.x + LABEL_INSET,
            cell.y + 2.0 * ICON_PAD + self.icon + LABEL_GAP,
            cell.width - 2.0 * LABEL_INSET,
            2.0 * LABEL_LINE,
        )
    }

    pub(crate) fn visible(&self, top: f32, height: f32) -> Vec<usize> {
        let mut shown = Vec::new();
        for section in &self.sections {
            let rows = section.rows(self.columns);
            let bottom = section.top + rows as f32 * self.cell_height;
            if bottom <= top || section.top >= top + height {
                continue;
            }
            let range = match section.row {
                true => {
                    let first = (section.shift / self.cell_width).floor().max(0.0) as usize;
                    let last = ((section.shift + self.usable) / self.cell_width).ceil() as usize;
                    first.min(section.len)..last.min(section.len)
                }
                false => {
                    let first = ((top - section.top) / self.cell_height).floor().max(0.0) as usize;
                    let last = ((top + height - section.top) / self.cell_height)
                        .ceil()
                        .max(0.0) as usize;
                    (first * self.columns).min(section.len)..(last * self.columns).min(section.len)
                }
            };
            shown.extend(range.map(|local| section.start + local));
        }
        shown
    }

    fn row_of(&self, index: usize) -> (usize, usize) {
        let section = self.section(index);
        let local = index - self.sections[section].start;
        match self.sections[section].row {
            true => (section, 0),
            false => (section, local / self.columns),
        }
    }

    fn nearest(&self, section: usize, row: usize, x: f32) -> usize {
        let section = self.sections[section];
        let (first, last) = match section.row {
            true => (0, section.len.saturating_sub(1)),
            false => (
                row * self.columns,
                ((row + 1) * self.columns)
                    .min(section.len)
                    .saturating_sub(1),
            ),
        };
        let start = self.cell(section.start + first).x;
        let offset = ((x - start) / self.cell_width).round().max(0.0) as usize;
        section.start + (first + offset).min(last)
    }

    fn vertical(&self, from: usize, down: bool) -> usize {
        let (section, row) = self.row_of(from);
        let x = self.cell(from).x;
        let target = match down {
            true if row + 1 < self.sections[section].rows(self.columns) => Some((section, row + 1)),
            true => (section + 1..self.sections.len())
                .find(|next| self.sections[*next].len > 0)
                .map(|next| (next, 0)),
            false if row > 0 => Some((section, row - 1)),
            false => (0..section)
                .rev()
                .find(|previous| self.sections[*previous].len > 0)
                .map(|previous| (previous, self.sections[previous].rows(self.columns) - 1)),
        };
        target.map_or(from, |(section, row)| self.nearest(section, row, x))
    }

    pub(crate) fn step(&self, from: Option<usize>, key: Key) -> Option<usize> {
        let last = self.count.checked_sub(1)?;
        let Some(from) = from else {
            return Some(match key {
                Key::Up | Key::Left | Key::End => last,
                _ => 0,
            });
        };
        Some(match key {
            Key::Left => from.saturating_sub(1),
            Key::Right => (from + 1).min(last),
            Key::Up => self.vertical(from, false),
            Key::Down => self.vertical(from, true),
            Key::Home => 0,
            Key::End => last,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Band {
    origin: Point,
    current: Point,
    base: Vec<usize>,
    toggle: bool,
}

fn normalised(a: Point, b: Point) -> Rect {
    Rect::new(
        a.x.min(b.x),
        a.y.min(b.y),
        (a.x - b.x).abs(),
        (a.y - b.y).abs(),
    )
}

fn intersects(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

fn clipped(rect: Rect, to: Rect) -> Rect {
    let (left, top) = (rect.x.max(to.x), rect.y.max(to.y));
    let right = (rect.x + rect.width).min(to.x + to.width);
    let bottom = (rect.y + rect.height).min(to.y + to.height);
    Rect::new(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
}

fn offset(rect: Rect, by: Point) -> Rect {
    Rect::new(rect.x + by.x, rect.y + by.y, rect.width, rect.height)
}

impl FilesView {
    pub(crate) fn grid(&mut self) -> Grid {
        self.sync_grid_location();
        let groups = self.groups();
        let count = self.browser.columns()[0].entries.len();
        let (width, icon) = (self.content_frame().width, self.icon_size as f32);
        let mut layouts = groups
            .iter()
            .map(|(start, title)| GroupLayout {
                start: *start,
                expanded: self.grid_scroll.expanded.contains(title),
                shift: 0.0,
            })
            .collect::<Vec<_>>();
        let grid = Grid::new(width, icon, count, &layouts);
        let viewport = grid.viewport_width();
        let cx = self.ui.context();
        for (section, (layout, (_, title))) in layouts.iter_mut().zip(groups.iter()).enumerate() {
            if !grid.is_row(section) {
                continue;
            }
            let state = self.grid_scroll.rows.entry(title.clone()).or_default();
            state.set_extent(grid.row_width(section), viewport);
            layout.shift = state.offset(cx);
        }
        Grid::new(width, icon, count, &layouts)
    }

    fn sync_grid_location(&mut self) {
        let location = self.browser.root().clone();
        if self.grid_scroll.location.as_ref() != Some(&location) {
            self.grid_scroll = GridScroll {
                location: Some(location),
                ..GridScroll::default()
            };
        }
    }

    pub(crate) fn thumbnail_size(&self) -> ThumbnailSize {
        ThumbnailSize::for_pixels(self.icon_size * 2)
    }

    fn fit_grid_scroll(&mut self) {
        let content = self.grid().content_height();
        let viewport = self.content_frame().height;
        self.grid_scroll.vertical.set_extent(content, viewport);
    }

    pub(crate) fn grid_offset(&mut self) -> f32 {
        self.fit_grid_scroll();
        let cx = self.ui.context();
        self.grid_scroll.vertical.offset(cx)
    }

    pub(crate) fn grid_origin(&mut self) -> Point {
        let content = self.content_frame();
        let scroll = self.grid_offset();
        Point::new(content.x, content.y - scroll)
    }

    pub(crate) fn reveal_in_grid(&mut self) {
        let Some(index) = self.browser.columns()[0].selected else {
            return;
        };
        self.fit_grid_scroll();
        let grid = self.grid();
        let groups = self.groups();
        let cx = self.ui.context();
        if let Some((_, title)) = grid.in_row(index).and_then(|section| groups.get(section)) {
            let (left, right) = grid.row_span(index);
            if let Some(state) = self.grid_scroll.rows.get_mut(title) {
                state.reveal(cx, left, right);
            }
        }
        let cell = grid.cell(index);
        self.grid_scroll
            .vertical
            .reveal(cx, cell.y - PADDING, cell.y + cell.height + PADDING);
    }

    fn row_under(&mut self, scroll: &Scroll) -> Option<String> {
        let sideways = scroll.delta.x.abs() > scroll.delta.y.abs();
        let origin = self.grid_origin();
        let section = self.grid().row_section(scroll.position.y - origin.y)?;
        let groups = self.groups();
        sideways.then(|| groups.get(section).map(|(_, title)| title.clone()))?
    }

    pub(crate) fn grid_scrolled(&mut self, scroll: &Scroll) -> bool {
        self.fit_grid_scroll();
        let row = match (scroll.source, scroll.phase) {
            (ScrollSource::Wheel, _) => self.row_under(scroll),
            (_, ScrollPhase::Began) => {
                self.grid_scroll.gesture = self.row_under(scroll);
                self.grid_scroll.gesture.clone()
            }
            (_, ScrollPhase::Ended) => self.grid_scroll.gesture.take(),
            (_, ScrollPhase::Changed) => self.grid_scroll.gesture.clone(),
        };
        let cx = self.ui.context();
        match row.and_then(|title| self.grid_scroll.rows.get_mut(&title)) {
            Some(state) => state.scroll(
                cx,
                &Scroll {
                    delta: Point::new(0.0, scroll.delta.x),
                    ..*scroll
                },
            ),
            None => self.grid_scroll.vertical.scroll(cx, scroll),
        }
    }

    pub(crate) fn toggle_group(&mut self, section: usize) {
        self.sync_grid_location();
        let groups = self.groups();
        let Some((_, title)) = groups.get(section) else {
            return;
        };
        if !self.grid_scroll.expanded.remove(title) {
            self.grid_scroll.expanded.insert(title.clone());
        }
        self.grid_scroll.rows.remove(title);
        self.fit_grid_scroll();
    }

    pub(crate) fn grid_key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        let selected = self.browser.columns()[0].selected;
        let Some(target) = self.grid().step(selected, key) else {
            return false;
        };
        if modifiers.shift {
            self.browser.extend_to(0, target);
        } else {
            self.browser.select(0, target);
        }
        self.selection_changed();
        true
    }

    pub(crate) fn grid_icon_centre(&mut self, row: usize) -> Point {
        let origin = self.grid_origin();
        let icon = offset(self.grid().icon_rect(row), origin);
        Point::new(icon.x + icon.width / 2.0, icon.y + icon.height / 2.0)
    }

    pub(crate) fn grid_rename_frame(&mut self, row: usize) -> Rect {
        let origin = self.grid_origin();
        let label = offset(self.grid().label_area(row), origin);
        Rect::new(label.x, label.y - 2.0, label.width, LABEL_LINE + 6.0)
    }

    pub(crate) fn start_band(&mut self, pointer: &Pointer) {
        let origin = self.grid_origin();
        let at = Point::new(pointer.position.x - origin.x, pointer.position.y - origin.y);
        self.press = Some(Press::Band);
        let keep = pointer.modifiers.shift || pointer.modifiers.control;
        if !keep {
            self.browser.focus_column(0);
        }
        self.band = Some(Band {
            origin: at,
            current: at,
            base: if keep {
                self.browser.columns()[0].marked.clone()
            } else {
                Vec::new()
            },
            toggle: pointer.modifiers.control,
        });
    }

    pub(crate) fn band_moved(&mut self, pointer: &Pointer) -> bool {
        let content = self.content_frame();
        let below = pointer.position.y - (content.y + content.height);
        let above = content.y - pointer.position.y;
        if below > 0.0 || above > 0.0 {
            let target = self.grid_offset() + below.max(0.0) - above.max(0.0);
            let cx = self.ui.context();
            self.grid_scroll.vertical.scroll_to(cx, target);
        }
        let origin = self.grid_origin();
        let grid = self.grid();
        let Some(band) = &mut self.band else {
            return false;
        };
        band.current = Point::new(pointer.position.x - origin.x, pointer.position.y - origin.y);
        let area = normalised(band.origin, band.current);
        let touched = grid
            .visible(area.y, area.height)
            .into_iter()
            .filter(|index| {
                intersects(grid.icon_rect(*index), area)
                    || intersects(grid.label_area(*index), area)
            })
            .collect::<Vec<_>>();
        let mut rows = band.base.clone();
        for index in touched {
            match rows.iter().position(|row| *row == index) {
                Some(found) if band.toggle => {
                    rows.remove(found);
                }
                Some(_) => {}
                None => rows.push(index),
            }
        }
        if rows != self.browser.columns()[0].marked {
            self.browser.mark(0, rows);
            self.selection_changed();
        }
        true
    }

    pub(crate) fn end_band(&mut self) -> bool {
        self.band.take().is_some()
    }

    fn grid_label(&mut self, name: &str, width: f32, color: Color) -> Paragraph {
        let shown = self.fitted_name(name, LABEL_SIZE, width, 2);
        self.text().paragraph(
            &shown,
            TextStyle::new(LABEL_SIZE, color),
            Some(width),
            TextAlign::Center,
        )
    }

    fn grid_picture(&mut self, entry: &Entry, size: ThumbnailSize) -> (Option<Image>, bool) {
        if let Some(image) = self.thumbs.get(entry, size) {
            return (Some(image), true);
        }
        let pixels = (self.icon_size * 2).min(256);
        (self.entry_icon(entry, pixels), false)
    }

    pub(crate) fn paint_grid(&mut self, items: &mut Vec<Item>) {
        let content = self.content_frame();
        self.hits.push((content, Hit::Column(0)));
        let grid = self.grid();
        let mut shown = Vec::new();
        let origin = self.grid_origin();
        let scroll = content.y - origin.y;
        let column = self.browser.columns()[0].clone();
        let count = column.entries.len();
        let visible = grid.visible(scroll, content.height);
        let groups = self.groups();
        for (index, y) in grid.headings() {
            let top = origin.y + y;
            if top + GROUP_HEADING < content.y || top > content.y + content.height {
                continue;
            }
            let Some((_, title)) = groups.get(index) else {
                continue;
            };
            let ink = self.palette.ink;
            let paragraph = self.text().paragraph(
                title,
                TextStyle::new(LABEL_SIZE + 1.0, ink).weight(TextStyle::SEMIBOLD),
                Some(content.width - 2.0 * PADDING),
                TextAlign::Start,
            );
            let baseline = top + GROUP_HEADING - 8.0;
            self.headings.push((
                clipped(
                    Rect::new(
                        content.x + PADDING,
                        top,
                        content.width - 2.0 * PADDING,
                        GROUP_HEADING,
                    ),
                    content,
                ),
                index,
            ));
            shown.push(Item::Text {
                origin: Point::new(
                    content.x + PADDING + LABEL_INSET,
                    baseline - paragraph.height(),
                ),
                paragraph,
            });
            shown.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    content.x + PADDING,
                    baseline + 3.0,
                    content.width - 2.0 * PADDING,
                    1.0,
                )),
                paint: self.palette.hairline.into(),
            });
            if grid.overflows(index) {
                self.paint_group_toggle(index, grid.is_row(index), content, baseline, &mut shown);
            }
        }
        let size = self.thumbnail_size();
        let renaming = self.rename.as_ref().map(|rename| rename.path.clone());
        for index in visible.iter().copied() {
            let entry = &column.entries[index];
            let marked = column.is_marked(index);
            let icon = offset(grid.icon_rect(index), origin);
            let label = offset(grid.label_area(index), origin);
            if marked {
                shown.push(Item::Fill {
                    shape: Shape::rounded(icon.inset(-ICON_PAD + 1.0), SELECTION_RADIUS),
                    paint: self.palette.well.into(),
                });
            }
            let (picture, thumbnail) = self.grid_picture(entry, size);
            if let Some(image) = picture {
                let fitted = crate::paint::fitted(icon, &image);
                shown.push(Item::Image {
                    rect: fitted,
                    image,
                });
                if thumbnail {
                    shown.push(Item::Stroke {
                        shape: Shape::rect(fitted.inset(-0.5)),
                        width: 1.0,
                        paint: self.palette.hairline.into(),
                    });
                }
            }
            self.hits.push((
                clipped(icon.inset(-ICON_PAD), content),
                Hit::Row {
                    column: 0,
                    row: index,
                },
            ));
            if renaming.as_ref() == Some(&entry.path) {
                continue;
            }
            let color = if marked {
                self.palette.on_accent
            } else {
                self.palette.ink
            };
            let name = self.shown_name(entry).to_owned();
            let paragraph = self.grid_label(&name, label.width, color);
            let (width, height) = (paragraph.width(), paragraph.height());
            let text_x = label.x + ((label.width - width) / 2.0).round();
            let text_area = Rect::new(text_x - 4.0, label.y - 1.0, width + 8.0, height + 2.0);
            if marked {
                shown.push(Item::Fill {
                    shape: Shape::rounded(text_area, SELECTION_RADIUS),
                    paint: self.palette.accent.into(),
                });
            }
            shown.push(Item::Text {
                origin: Point::new(label.x, label.y),
                paragraph,
            });
            self.hits.push((
                clipped(text_area, content),
                Hit::Row {
                    column: 0,
                    row: index,
                },
            ));
        }
        if column.entries.is_empty() {
            self.paint_empty(0, content, &mut shown);
        }
        if let Some(band) = &self.band {
            let area = offset(normalised(band.origin, band.current), origin);
            shown.push(Item::Fill {
                shape: Shape::rect(area),
                paint: self.palette.accent_soft.with_opacity(0.6).into(),
            });
            shown.push(Item::Stroke {
                shape: Shape::rect(area.inset(0.5)),
                width: 1.0,
                paint: self.palette.accent_text.into(),
            });
        }
        items.push(Item::Clip {
            shape: Shape::rect(content),
            items: shown,
        });
        let cx = self.ui.context();
        self.grid_scroll
            .vertical
            .paint_scrollbar(cx, content, items);
        let end = visible.last().map_or(0, |last| last + 1);
        let ahead = end..(end + visible.len()).min(count);
        self.grid_wanted = visible
            .iter()
            .filter_map(|index| column.entries.get(*index))
            .chain(&column.entries[ahead])
            .cloned()
            .collect();
    }

    fn paint_group_toggle(
        &mut self,
        section: usize,
        collapsed: bool,
        content: Rect,
        baseline: f32,
        items: &mut Vec<Item>,
    ) {
        let hovered = self.hover == Some(Hit::GroupToggle(section));
        let color = match hovered {
            true => self.palette.ink,
            false => self.palette.ink2,
        };
        let title = L10N.tr(match collapsed {
            true => "group-show-all",
            false => "group-show-less",
        });
        let paragraph = self.text().paragraph(
            &title,
            TextStyle::new(LABEL_SIZE, color).weight(TextStyle::MEDIUM),
            None,
            TextAlign::Start,
        );
        let right = content.x + content.width - PADDING - LABEL_INSET;
        let origin = Point::new(
            (right - paragraph.width()).round(),
            baseline - paragraph.height(),
        );
        let area = Rect::new(
            origin.x - LABEL_INSET,
            origin.y - 2.0,
            paragraph.width() + 2.0 * LABEL_INSET,
            paragraph.height() + 4.0,
        );
        items.push(Item::Text { origin, paragraph });
        self.hits
            .push((clipped(area, content), Hit::GroupToggle(section)));
    }
}
