use std::ops::Range;

use bande_fs::Entry;
use bande_thumbnails::ThumbnailSize;
use bande_toolkit::ui::{Key, Modifiers, Pointer, Scroll, ScrollState};
use bande_toolkit::{
    Color, Image, Item, Paragraph, Point, Radius, Rect, Shape, TextAlign, TextStyle,
};

use crate::L10N;
use crate::controls::Press;
use crate::view::{FilesView, Hit};

const PADDING: f32 = 14.0;
const ICON_PAD: f32 = 6.0;
const LABEL_SIZE: f32 = 12.0;
const LABEL_LINE: f32 = 16.0;
const LABEL_GAP: f32 = 4.0;
const LABEL_INSET: f32 = 4.0;
const SELECTION_RADIUS: f32 = Radius::S;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Grid {
    pub columns: usize,
    pub cell_width: f32,
    pub cell_height: f32,
    pub icon: f32,
}

impl Grid {
    pub(crate) fn new(width: f32, icon: f32) -> Self {
        let minimum = (icon + 36.0).max(96.0);
        let usable = (width - 2.0 * PADDING).max(minimum);
        let columns = ((usable / minimum).floor() as usize).max(1);
        Self {
            columns,
            cell_width: (usable / columns as f32).floor(),
            cell_height: (2.0 * ICON_PAD + icon + LABEL_GAP + 2.0 * LABEL_LINE + 6.0).ceil(),
            icon,
        }
    }

    pub(crate) fn content_height(&self, count: usize) -> f32 {
        count.div_ceil(self.columns) as f32 * self.cell_height + 2.0 * PADDING
    }

    pub(crate) fn cell(&self, index: usize) -> Rect {
        let (row, column) = (index / self.columns, index % self.columns);
        Rect::new(
            PADDING + column as f32 * self.cell_width,
            PADDING + row as f32 * self.cell_height,
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

    pub(crate) fn visible(&self, top: f32, height: f32, count: usize) -> Range<usize> {
        let first_row = ((top - PADDING).max(0.0) / self.cell_height).floor() as usize;
        let last_row = ((top + height - PADDING).max(0.0) / self.cell_height).ceil() as usize;
        (first_row * self.columns).min(count)..(last_row * self.columns).min(count)
    }

    pub(crate) fn step(&self, from: Option<usize>, key: Key, count: usize) -> Option<usize> {
        let last = count.checked_sub(1)?;
        let Some(from) = from else {
            return Some(match key {
                Key::Up | Key::Left | Key::End => last,
                _ => 0,
            });
        };
        Some(match key {
            Key::Left => from.saturating_sub(1),
            Key::Right => (from + 1).min(last),
            Key::Up if from >= self.columns => from - self.columns,
            Key::Up => from,
            Key::Down if from + self.columns <= last => from + self.columns,
            Key::Down if from / self.columns < last / self.columns => last,
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
    pub(crate) fn grid(&self) -> Grid {
        Grid::new(self.content_frame().width, self.icon_size as f32)
    }

    pub(crate) fn thumbnail_size(&self) -> ThumbnailSize {
        ThumbnailSize::for_pixels(self.icon_size * 2)
    }

    fn fit_grid_scroll(&mut self) {
        let location = self.browser.root().clone();
        if self.grid_scroll.0.as_ref() != Some(&location) {
            self.grid_scroll = (Some(location), ScrollState::default());
        }
        let count = self.browser.columns()[0].entries.len();
        let content = self.grid().content_height(count);
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
        let column = &self.browser.columns()[0];
        let Some(target) = self.grid().step(column.selected, key, column.entries.len()) else {
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
        let keep = pointer.modifiers.shift || pointer.modifiers.logo;
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
            toggle: pointer.modifiers.logo,
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
        let Some(band) = &mut self.band else {
            return false;
        };
        band.current = Point::new(pointer.position.x - origin.x, pointer.position.y - origin.y);
        let area = normalised(band.origin, band.current);
        let grid = Grid::new(content.width, self.icon_size as f32);
        let count = self.browser.columns()[0].entries.len();
        let touched = grid
            .visible(area.y, area.height, count)
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
        let origin = self.grid_origin();
        let scroll = content.y - origin.y;
        let column = self.browser.columns()[0].clone();
        let count = column.entries.len();
        let visible = grid.visible(scroll, content.height, count);
        let size = self.thumbnail_size();
        let renaming = self.rename.as_ref().map(|rename| rename.path.clone());
        let mut shown = Vec::new();
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
            let paragraph = self.grid_label(&entry.name, label.width, color);
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
            let muted = self.palette.ink2;
            let empty = self.text().paragraph(
                &L10N.tr("empty-folder"),
                TextStyle::new(13.0, muted),
                Some(content.width),
                TextAlign::Center,
            );
            shown.push(Item::Text {
                origin: Point::new(content.x, content.y + 40.0),
                paragraph: empty,
            });
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
