use std::ops::Range;

use bande_fs::Entry;
use bande_thumbnails::ThumbnailSize;
use bande_toolkit::ui::{Key, Modifiers, Pointer, Scroll, ScrollState};
use bande_toolkit::{
    Color, Image, Item, Paragraph, Point, Radius, Rect, Shape, TextAlign, TextStyle,
};

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

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Grid {
    pub columns: usize,
    pub cell_width: f32,
    pub cell_height: f32,
    pub icon: f32,
    count: usize,
    sections: Vec<(usize, f32)>,
}

impl Grid {
    pub(crate) fn new(width: f32, icon: f32, count: usize, starts: &[usize]) -> Self {
        let minimum = (icon + 36.0).max(96.0);
        let usable = (width - 2.0 * PADDING).max(minimum);
        let columns = ((usable / minimum).floor() as usize).max(1);
        let cell_height = (2.0 * ICON_PAD + icon + LABEL_GAP + 2.0 * LABEL_LINE + 6.0).ceil();
        let mut sections = Vec::with_capacity(starts.len().max(1));
        let mut top = PADDING;
        for (index, start) in starts.iter().enumerate() {
            let end = starts.get(index + 1).copied().unwrap_or(count);
            sections.push((*start, top + GROUP_HEADING));
            top += GROUP_HEADING + (end - start).div_ceil(columns) as f32 * cell_height;
        }
        if sections.is_empty() {
            sections.push((0, PADDING));
        }
        Self {
            columns,
            cell_width: (usable / columns as f32).floor(),
            cell_height,
            icon,
            count,
            sections,
        }
    }

    fn section(&self, index: usize) -> usize {
        self.sections
            .partition_point(|(start, _)| *start <= index)
            .saturating_sub(1)
    }

    fn section_len(&self, section: usize) -> usize {
        let end = self
            .sections
            .get(section + 1)
            .map_or(self.count, |(start, _)| *start);
        end.saturating_sub(self.sections[section].0)
    }

    pub(crate) fn headings(&self) -> impl Iterator<Item = (usize, f32)> + '_ {
        self.sections
            .iter()
            .enumerate()
            .filter(|_| self.sections.len() > 1 || self.sections[0].1 > PADDING)
            .map(|(index, (_, top))| (index, top - GROUP_HEADING))
    }

    pub(crate) fn content_height(&self) -> f32 {
        let last = self.sections.len() - 1;
        let (_, top) = self.sections[last];
        top + self.section_len(last).div_ceil(self.columns) as f32 * self.cell_height + PADDING
    }

    pub(crate) fn cell(&self, index: usize) -> Rect {
        let (start, top) = self.sections[self.section(index)];
        let local = index - start;
        let (row, column) = (local / self.columns, local % self.columns);
        Rect::new(
            PADDING + column as f32 * self.cell_width,
            top + row as f32 * self.cell_height,
            self.cell_width,
            self.cell_height,
        )
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

    fn first_where(&self, below: impl Fn(Rect) -> bool) -> usize {
        let (mut low, mut high) = (0, self.count);
        while low < high {
            let middle = (low + high) / 2;
            match below(self.cell(middle)) {
                true => low = middle + 1,
                false => high = middle,
            }
        }
        low
    }

    pub(crate) fn visible(&self, top: f32, height: f32) -> Range<usize> {
        let first = self.first_where(|cell| cell.y + cell.height <= top);
        let last = self.first_where(|cell| cell.y < top + height);
        first..last.max(first)
    }

    pub(crate) fn step(&self, from: Option<usize>, key: Key) -> Option<usize> {
        let last = self.count.checked_sub(1)?;
        let Some(from) = from else {
            return Some(match key {
                Key::Up | Key::Left | Key::End => last,
                _ => 0,
            });
        };
        let section = self.section(from);
        let (start, _) = self.sections[section];
        let (local, length) = (from - start, self.section_len(section));
        let column = local % self.columns;
        Some(match key {
            Key::Left => from.saturating_sub(1),
            Key::Right => (from + 1).min(last),
            Key::Up if local >= self.columns => from - self.columns,
            Key::Up if section > 0 => {
                let (above, _) = self.sections[section - 1];
                let length = self.section_len(section - 1);
                let row = length.saturating_sub(1) / self.columns;
                above + (row * self.columns + column).min(length.saturating_sub(1))
            }
            Key::Up => from,
            Key::Down if local + self.columns < length => from + self.columns,
            Key::Down if local / self.columns < length.saturating_sub(1) / self.columns => {
                start + length - 1
            }
            Key::Down if section + 1 < self.sections.len() => {
                let (below, _) = self.sections[section + 1];
                below + column.min(self.section_len(section + 1).saturating_sub(1))
            }
            Key::Down => from,
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

fn offset(rect: Rect, by: Point) -> Rect {
    Rect::new(rect.x + by.x, rect.y + by.y, rect.width, rect.height)
}

impl FilesView {
    pub(crate) fn grid(&mut self) -> Grid {
        let starts = self
            .groups()
            .iter()
            .map(|(start, _)| *start)
            .collect::<Vec<_>>();
        let count = self.browser.columns()[0].entries.len();
        Grid::new(
            self.content_frame().width,
            self.icon_size as f32,
            count,
            &starts,
        )
    }

    pub(crate) fn thumbnail_size(&self) -> ThumbnailSize {
        ThumbnailSize::for_pixels(self.icon_size * 2)
    }

    fn fit_grid_scroll(&mut self) {
        let location = self.browser.root().clone();
        if self.grid_scroll.0.as_ref() != Some(&location) {
            self.grid_scroll = (Some(location), ScrollState::default());
        }
        let content = self.grid().content_height();
        let viewport = self.content_frame().height;
        self.grid_scroll.1.set_extent(content, viewport);
    }

    pub(crate) fn grid_offset(&mut self) -> f32 {
        self.fit_grid_scroll();
        let cx = self.ui.context();
        self.grid_scroll.1.offset(cx)
    }

    pub(crate) fn grid_origin(&mut self) -> Point {
        let content = self.content_frame();
        let scroll = self.grid_offset();
        Point::new(content.x, content.y - scroll)
    }

    pub(crate) fn reveal_in_grid(&mut self) {
        let Some(row) = self.browser.columns()[0].selected else {
            return;
        };
        self.fit_grid_scroll();
        let cell = self.grid().cell(row);
        let cx = self.ui.context();
        self.grid_scroll
            .1
            .reveal(cx, cell.y - PADDING, cell.y + cell.height + PADDING);
    }

    pub(crate) fn grid_scrolled(&mut self, scroll: &Scroll) -> bool {
        self.fit_grid_scroll();
        let cx = self.ui.context();
        self.grid_scroll.1.scroll(cx, scroll)
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
            self.grid_scroll.1.scroll_to(cx, target);
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
        }
        let size = self.thumbnail_size();
        let renaming = self.rename.as_ref().map(|rename| rename.path.clone());
        for index in visible.clone() {
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
                icon.inset(-ICON_PAD),
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
                text_area,
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
        self.grid_scroll.1.paint_scrollbar(cx, content, items);
        let ahead = visible.end..(visible.end + visible.len()).min(count);
        self.grid_wanted = column.entries[visible]
            .iter()
            .chain(&column.entries[ahead])
            .cloned()
            .collect();
    }
}
