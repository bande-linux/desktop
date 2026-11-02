use std::ops::Range;

use bande_fs::Entry;
use bande_thumbnails::ThumbnailSize;
use bande_toolkit::ui::{Key, Modifiers};
use bande_toolkit::{Item, Point, Radius, Rect, Shape, TextAlign, TextRole, TextStyle};

use crate::paint::{centred, fitted};
use crate::view::{FilesView, Hit};

const STRIP_HEIGHT: f32 = 88.0;
const THUMB: f32 = 56.0;
const CELL: f32 = 68.0;
const PADDING: f32 = 16.0;
const NAME_HEIGHT: f32 = 30.0;
const ICON_PIXELS: u32 = 256;
const RING: f32 = 3.0;

pub(crate) fn strip_size() -> ThumbnailSize {
    ThumbnailSize::for_pixels((THUMB * 2.0) as u32)
}

impl FilesView {
    fn gallery_preview_area(&self) -> Rect {
        let content = self.content_frame();
        Rect::new(
            content.x + PADDING,
            content.y + PADDING,
            (content.width - 2.0 * PADDING).max(0.0),
            (content.height - STRIP_HEIGHT - NAME_HEIGHT - 2.0 * PADDING).max(0.0),
        )
    }

    pub(crate) fn gallery_name_frame(&self) -> Rect {
        let preview = self.gallery_preview_area();
        Rect::new(
            preview.x,
            preview.y + preview.height + 4.0,
            preview.width,
            NAME_HEIGHT - 6.0,
        )
    }

    fn gallery_strip_start(&self, count: usize, selected: Option<usize>) -> f32 {
        let content = self.content_frame();
        let total = count as f32 * CELL;
        let room = content.width - 2.0 * PADDING;
        if total <= room {
            return content.x + ((content.width - total) / 2.0).round();
        }
        let centre = selected.map_or(CELL / 2.0, |row| row as f32 * CELL + CELL / 2.0);
        (content.x + content.width / 2.0 - centre)
            .clamp(
                content.x + content.width - PADDING - total,
                content.x + PADDING,
            )
            .round()
    }

    fn gallery_visible(&self, start: f32, count: usize) -> Range<usize> {
        let content = self.content_frame();
        let first = ((content.x - start) / CELL).floor().max(0.0) as usize;
        let last = ((content.x + content.width - start) / CELL).ceil().max(0.0) as usize;
        first.min(count)..last.min(count)
    }

    pub(crate) fn gallery_key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        let column = &self.browser.columns()[0];
        let Some(last) = column.entries.len().checked_sub(1) else {
            return false;
        };
        let target = match (column.selected, key) {
            (_, Key::Home) => 0,
            (_, Key::End) => last,
            (None, Key::Left | Key::Up) => last,
            (None, Key::Right | Key::Down) => 0,
            (Some(from), Key::Left | Key::Up) => from.saturating_sub(1),
            (Some(from), Key::Right | Key::Down) => (from + 1).min(last),
            _ => return false,
        };
        if modifiers.shift {
            self.browser.extend_to(0, target);
        } else {
            self.browser.select(0, target);
        }
        self.selection_changed();
        true
    }

    fn paint_gallery_preview(&mut self, entry: &Entry, row: usize, items: &mut Vec<Item>) {
        let area = self.gallery_preview_area();
        let picture = match self.thumbs.get(entry, ThumbnailSize::XLarge) {
            Some(image) => Some((fitted(area, &image), image, true)),
            None => self.entry_icon(entry, ICON_PIXELS).map(|image| {
                let side = (ICON_PIXELS as f32 / 2.0).min(area.width).min(area.height);
                (centred(area, side, side), image, false)
            }),
        };
        if let Some((rect, image, thumbnail)) = picture {
            items.push(Item::Image { rect, image });
            if thumbnail {
                items.push(Item::Stroke {
                    shape: Shape::rect(rect.inset(-0.5)),
                    width: 1.0,
                    paint: self.palette.hairline.into(),
                });
            }
            self.hits.push((rect, Hit::Row { column: 0, row }));
        }
        if self.renaming() == Some(entry.path.as_path()) {
            return;
        }
        let frame = self.gallery_name_frame();
        let name = self.shown_name(entry).to_owned();
        let shown = self.fitted_name(&name, 13.0, frame.width, 1);
        let ink = self.palette.ink;
        let paragraph = self.text().paragraph(
            &shown,
            TextStyle::role(TextRole::BodyStrong, ink),
            Some(frame.width),
            TextAlign::Center,
        );
        let y = frame.y + ((frame.height - paragraph.height()) / 2.0).round();
        items.push(Item::Text {
            origin: Point::new(frame.x, y),
            paragraph,
        });
    }

    pub(crate) fn paint_gallery(&mut self, items: &mut Vec<Item>) {
        let content = self.content_frame();
        self.hits.push((content, Hit::Column(0)));
        let column = self.browser.columns()[0].clone();
        let mut shown = Vec::new();
        if column.entries.is_empty() {
            self.paint_empty(0, content, &mut shown);
        }
        if let Some(row) = column.selected
            && let Some(entry) = column.entries.get(row)
        {
            self.paint_gallery_preview(entry, row, &mut shown);
        }
        let strip_y = content.y + content.height - STRIP_HEIGHT;
        shown.push(Item::Fill {
            shape: Shape::rect(Rect::new(content.x, strip_y, content.width, 1.0)),
            paint: self.palette.hairline.into(),
        });
        let count = column.entries.len();
        let start = self.gallery_strip_start(count, column.selected);
        let visible = self.gallery_visible(start, count);
        let size = strip_size();
        for index in visible.clone() {
            let entry = &column.entries[index];
            let cell = Rect::new(
                start + index as f32 * CELL,
                strip_y + 1.0,
                CELL,
                STRIP_HEIGHT - 1.0,
            );
            let well = centred(cell, THUMB + 2.0 * RING, THUMB + 2.0 * RING);
            if column.is_marked(index) {
                shown.push(Item::Stroke {
                    shape: Shape::rounded(well.inset(RING / 2.0), Radius::S),
                    width: RING,
                    paint: self.palette.accent.into(),
                });
            }
            let thumb = centred(cell, THUMB, THUMB);
            let picture = match self.thumbs.get(entry, size) {
                Some(image) => Some(image),
                None => self.entry_icon(entry, (THUMB * 2.0) as u32),
            };
            if let Some(image) = picture {
                shown.push(Item::Image {
                    rect: fitted(thumb, &image),
                    image,
                });
            }
            self.hits.push((
                well,
                Hit::Row {
                    column: 0,
                    row: index,
                },
            ));
        }
        items.push(Item::Clip {
            shape: Shape::rect(content),
            items: shown,
        });
        let wanted = column.entries[visible].to_vec();
        self.grid_wanted = wanted;
    }
}
