use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use unbound_fs::{Entry, Purpose, format_date, format_size};
use unbound_preview::Preview;
use unbound_thumbnails::ThumbnailSize;
use unbound_toolkit::{
    Color, Elevation, Image, Item, Paragraph, Point, Radius, Rect, Scene, Shape, TextAlign,
    TextRole, TextStyle,
};

use crate::L10N;
use crate::browser::Location;
use crate::sidebar::{PlaceKind, Row};
use crate::toolbar::TOOLBAR_HEIGHT;
use crate::view::{
    COLUMN_WIDTH, FilesView, Hit, INSPECTOR_WIDTH, PATH_BAR_HEIGHT, PROGRESS_HEIGHT, QuickLook,
    ROW_HEIGHT, SIDEBAR_WIDTH,
};
use crate::views::ViewMode;

const ROW_INSET: f32 = 4.0;
const NAME_SIZE: f32 = 13.0;
const SMALL_SIZE: f32 = 11.0;
const CODE_SIZE: f32 = 11.0;
const ICON_SIZE: f32 = 16.0;
const ICON_PIXELS: u32 = 32;
const LARGE_ICON_PIXELS: u32 = 256;
const SIDEBAR_ROW: f32 = 28.0;
const SIDEBAR_PITCH: f32 = 30.0;
const SIDEBAR_INSET: f32 = 8.0;
const SIDEBAR_TEXT_X: f32 = 20.0;
const PROGRESS_CELLS: usize = 24;
const PROGRESS_GAP: f32 = 2.0;
const QUICK_LOOK_SHARE: f32 = 0.8;
const QUICK_LOOK_HEADER: f32 = 36.0;
const PADDING: f32 = 16.0;
const BUTTON_HEIGHT: f32 = 24.0;

pub fn permissions(mode: u32) -> String {
    (0..9)
        .map(|bit| {
            let set = mode & (0o400 >> bit) != 0;
            match (set, bit % 3) {
                (false, _) => '-',
                (true, 0) => 'r',
                (true, 1) => 'w',
                (true, _) => 'x',
            }
        })
        .collect()
}

fn centred(area: Rect, width: f32, height: f32) -> Rect {
    Rect::new(
        area.x + ((area.width - width) / 2.0).round(),
        area.y + ((area.height - height) / 2.0).round(),
        width,
        height,
    )
}

pub(crate) fn fitted(area: Rect, image: &Image) -> Rect {
    let (width, height) = (image.width() as f32, image.height() as f32);
    let scale = (area.width / width).min(area.height / height).min(1.0);
    centred(area, (width * scale).round(), (height * scale).round())
}

impl FilesView {
    fn paragraph(&mut self, text: &str, size: f32, color: Color, weight: f32) -> Paragraph {
        self.text().paragraph(
            text,
            TextStyle::new(size, color).weight(weight),
            None,
            TextAlign::Start,
        )
    }

    fn label(&mut self, text: &str, size: f32, color: Color) -> Paragraph {
        self.paragraph(text, size, color, TextStyle::REGULAR)
    }

    fn line(&mut self, text: &str, size: f32, color: Color, x: f32, y: f32, height: f32) -> Item {
        let paragraph = self.label(text, size, color);
        Item::Text {
            origin: Point::new(x, y + ((height - paragraph.height()) / 2.0).round()),
            paragraph,
        }
    }

    pub(crate) fn entry_icon(&mut self, entry: &Entry, pixels: u32) -> Option<Image> {
        if entry.is_folder() {
            return self.icons.first(&["folder"], pixels);
        }
        let names = self.apps.icon_names_for_name(&entry.name);
        let names = names.iter().map(String::as_str).collect::<Vec<_>>();
        self.icons.first(&names, pixels)
    }

    fn fill(&self, rect: Rect, radius: f32, color: Color) -> Item {
        Item::Fill {
            shape: Shape::rounded(rect, radius),
            paint: color.into(),
        }
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        self.size = (width as f32, height as f32);
        self.hits.clear();
        self.layout_controls();
        self.refresh_navigation();
        let (controls, sheet) = self.ui.paint_layers(width, height);
        let sheet_shape = Shape::rounded(self.sheet_frame(), self.sheet_radius());
        let mut items = Vec::new();
        self.paint_sidebar(&mut items);
        let mut content = Vec::new();
        self.paint_columns(&mut content);
        self.paint_scope_bar(&mut content);
        self.paint_tab_bar(&mut content);
        self.paint_path_bar(&mut content);
        items.push(Item::Clip {
            shape: sheet_shape,
            items: content,
        });
        items.extend(controls);
        self.paint_drop(&mut items);
        let mut overlays = Vec::new();
        self.paint_progress(&mut overlays);
        if self.inspector {
            self.paint_inspector(&mut overlays);
        }
        items.push(Item::Clip {
            shape: sheet_shape,
            items: overlays,
        });
        self.paint_notice(&mut items);
        self.paint_quick_look(&mut items);
        items.extend(sheet);
        self.want_thumbnails();
        Scene {
            width,
            height,
            background: self.palette.sheet,
            items,
        }
    }

    fn sheet_radius(&self) -> f32 {
        match self.sheet_inset > 0.0 {
            true => Radius::concentric(Radius::XL, self.sheet_inset),
            false => 0.0,
        }
    }

    fn want_thumbnails(&mut self) {
        let mut wanted = Vec::new();
        if let Some(QuickLook {
            entry,
            preview: Some(Preview::Other { .. }),
        }) = &self.quick_look
        {
            wanted.push((entry, ThumbnailSize::XLarge));
        }
        if let Some(info) = &self.info
            && let ([entry], Some(Preview::Other { .. })) = (info.entries.as_slice(), &info.preview)
        {
            wanted.push((entry, ThumbnailSize::Large));
        }
        if self.mode == ViewMode::Icons {
            let size = self.thumbnail_size();
            wanted.extend(self.grid_wanted.iter().map(|entry| (entry, size)));
        }
        let apps = &self.apps;
        self.thumbs.want(wanted, |entry| apps.mime_for(&entry.path));
    }

    fn paint_sidebar(&mut self, items: &mut Vec<Item>) {
        let palette = self.palette;
        let root = self.browser.root().clone();
        let trash_full = self.trash_full;
        self.hits.push((
            Rect::new(
                0.0,
                TOOLBAR_HEIGHT,
                SIDEBAR_WIDTH,
                self.size.1 - TOOLBAR_HEIGHT,
            ),
            Hit::Sidebar,
        ));
        let mut y = TOOLBAR_HEIGHT + 4.0;
        for (index, row) in self.sidebar.clone().iter().enumerate() {
            match row {
                Row::Heading(title) => {
                    y += 8.0;
                    let paragraph = self.text().paragraph(
                        title,
                        TextStyle::role(TextRole::Label, palette.ink3),
                        None,
                        TextAlign::Start,
                    );
                    items.push(Item::Text {
                        origin: Point::new(SIDEBAR_TEXT_X, y + 4.0),
                        paragraph,
                    });
                    y += 24.0;
                }
                Row::Place(place) => {
                    let rect = Rect::new(
                        SIDEBAR_INSET,
                        y,
                        SIDEBAR_WIDTH - 2.0 * SIDEBAR_INSET,
                        SIDEBAR_ROW,
                    );
                    let selected = place.location == root;
                    if selected {
                        items.push(self.fill(rect, Radius::S, palette.well));
                    } else if self.hover == Some(Hit::Place(index)) {
                        items.push(self.fill(rect, Radius::S, palette.well.with_opacity(0.5)));
                    }
                    let icon = match (&place.location, trash_full) {
                        (Location::Trash, true) => "user-trash-full",
                        _ => place.icon,
                    };
                    let tint = match selected {
                        true => palette.accent,
                        false => palette.ink2,
                    };
                    let symbol = format!("{icon}-symbolic");
                    if let Some(image) =
                        self.icons
                            .symbol(&[&symbol, "folder-symbolic"], ICON_PIXELS, tint)
                    {
                        items.push(Item::Image {
                            rect: Rect::new(
                                SIDEBAR_TEXT_X,
                                y + ((SIDEBAR_ROW - ICON_SIZE) / 2.0).round(),
                                ICON_SIZE,
                                ICON_SIZE,
                            ),
                            image,
                        });
                    }
                    let text = self.line(
                        &place.name,
                        NAME_SIZE,
                        palette.ink,
                        SIDEBAR_TEXT_X + ICON_SIZE + 8.0,
                        y,
                        SIDEBAR_ROW,
                    );
                    items.push(Item::Clip {
                        shape: Shape::rect(rect),
                        items: vec![text],
                    });
                    self.hits.push((rect, Hit::Place(index)));
                    if place.kind == PlaceKind::Volume {
                        let button = Rect::new(
                            rect.x + rect.width - ICON_SIZE - 6.0,
                            y + ((SIDEBAR_ROW - ICON_SIZE) / 2.0).round(),
                            ICON_SIZE,
                            ICON_SIZE,
                        );
                        if let Some(image) =
                            self.icons
                                .symbol(&["media-eject-symbolic"], ICON_PIXELS, palette.ink2)
                        {
                            items.push(Item::Image {
                                rect: button,
                                image,
                            });
                        }
                        self.hits.push((button.inset(-4.0), Hit::Eject(index)));
                    }
                    y += SIDEBAR_PITCH;
                }
            }
        }
    }

    fn paint_columns(&mut self, items: &mut Vec<Item>) {
        let content = self.content_frame();
        match self.mode {
            ViewMode::Icons => return self.paint_grid(items),
            ViewMode::List => return,
            ViewMode::Columns => {}
        }
        let first = self.first_visible_column();
        if let Some(last) = self.browser.columns().len().checked_sub(1) {
            self.hits.push((content, Hit::Column(last)));
        }
        for index in first..self.browser.columns().len() {
            let x = content.x + (index - first) as f32 * COLUMN_WIDTH;
            if x >= content.x + content.width {
                break;
            }
            self.paint_column(index, x, content, items);
        }
    }

    fn paint_column(&mut self, index: usize, x: f32, content: Rect, items: &mut Vec<Item>) {
        let active = index == self.browser.active();
        let frame = Rect::new(x, content.y, COLUMN_WIDTH, content.height);
        self.hits.push((frame, Hit::Column(index)));
        let mut rows = Vec::new();
        let scroll = self.column_offset(index);
        let first = (scroll.max(0.0) / ROW_HEIGHT).floor() as usize;
        let fitting = (content.height / ROW_HEIGHT).ceil() as usize + 2;
        let column = &self.browser.columns()[index];
        let empty = column.entries.is_empty();
        let shown = column
            .entries
            .iter()
            .enumerate()
            .skip(first)
            .take(fitting)
            .map(|(position, entry)| (position, entry.clone(), column.is_marked(position)))
            .collect::<Vec<_>>();
        let renaming = self.rename.as_ref().map(|rename| rename.path.clone());
        for (position, entry, marked) in &shown {
            let (position, marked) = (*position, *marked);
            let y = content.y + self.rows_top() + position as f32 * ROW_HEIGHT - scroll;
            if y > content.y + content.height {
                break;
            }
            let row = Rect::new(x + ROW_INSET, y, COLUMN_WIDTH - 2.0 * ROW_INSET, ROW_HEIGHT);
            self.hits.push((
                Rect::new(x, y, COLUMN_WIDTH, ROW_HEIGHT),
                Hit::Row {
                    column: index,
                    row: position,
                },
            ));
            let color = if marked && active {
                self.palette.on_accent
            } else {
                self.palette.ink
            };
            if marked {
                rows.push(self.fill(
                    Rect::new(row.x, row.y + 1.0, row.width, row.height - 2.0),
                    Radius::S,
                    if active {
                        self.palette.accent
                    } else {
                        self.palette.well
                    },
                ));
            }
            if let Some(image) = self.entry_icon(entry, ICON_PIXELS) {
                rows.push(Item::Image {
                    rect: Rect::new(
                        row.x + 6.0,
                        y + ((ROW_HEIGHT - ICON_SIZE) / 2.0).round(),
                        ICON_SIZE,
                        ICON_SIZE,
                    ),
                    image,
                });
            }
            let name_x = row.x + 28.0;
            if renaming.as_ref() == Some(&entry.path) {
                continue;
            }
            let name_width = row.width - 28.0 - if entry.is_folder() { 22.0 } else { 6.0 };
            let name = self.line(&entry.name, NAME_SIZE, color, name_x, y, ROW_HEIGHT);
            rows.push(Item::Clip {
                shape: Shape::rect(Rect::new(name_x, y, name_width, ROW_HEIGHT)),
                items: vec![name],
            });
            if entry.is_folder() {
                let chevron_color = match marked && active {
                    true => color,
                    false => self.palette.ink3,
                };
                let chevron = self.label("\u{203A}", NAME_SIZE + 3.0, chevron_color);
                let chevron_y = y + ((ROW_HEIGHT - chevron.height()) / 2.0).round();
                rows.push(Item::Text {
                    origin: Point::new(row.x + row.width - 8.0 - chevron.width(), chevron_y),
                    paragraph: chevron,
                });
            }
        }
        if empty {
            let muted = self.palette.ink2;
            let empty = self.text().paragraph(
                &L10N.tr("empty-folder"),
                TextStyle::new(NAME_SIZE, muted),
                Some(COLUMN_WIDTH),
                TextAlign::Center,
            );
            rows.push(Item::Text {
                origin: Point::new(x, content.y + 40.0),
                paragraph: empty,
            });
        }
        items.push(Item::Clip {
            shape: Shape::rect(frame),
            items: rows,
        });
        self.paint_column_scrollbar(index, frame, items);
        items.push(self.fill(
            Rect::new(x + COLUMN_WIDTH - 1.0, content.y, 1.0, content.height),
            0.0,
            self.palette.hairline,
        ));
    }

    fn shown_path(&self) -> Option<PathBuf> {
        match self.browser.selected() {
            Some(entry) => Some(entry.path.clone()),
            None => self.browser.active_folder().map(Path::to_path_buf),
        }
    }

    fn paint_path_bar(&mut self, items: &mut Vec<Item>) {
        if !self.status_bar {
            return;
        }
        let content = self.content_frame();
        let sheet = self.sheet_frame();
        let bar = Rect::new(
            content.x,
            sheet.y + sheet.height - PATH_BAR_HEIGHT,
            content.width,
            PATH_BAR_HEIGHT,
        );
        items.push(self.fill(
            Rect::new(bar.x + 12.0, bar.y, (bar.width - 24.0).max(0.0), 1.0),
            0.0,
            self.palette.hairline,
        ));
        let status = self.status_text();
        let muted = self.palette.ink2;
        let status_paragraph = self.text().paragraph(
            &status,
            TextStyle::role(TextRole::Callout, muted).tabular(),
            None,
            TextAlign::Start,
        );
        let slider = match self.size_slider {
            Some(_) => crate::controls::SLIDER_WIDTH + 26.0,
            None => 0.0,
        };
        let status_x = bar.x + bar.width - status_paragraph.width() - 20.0 - slider;
        items.push(Item::Text {
            origin: Point::new(
                status_x,
                bar.y + ((PATH_BAR_HEIGHT - status_paragraph.height()) / 2.0).round(),
            ),
            paragraph: status_paragraph,
        });
        let mut segments = Vec::new();
        if self.browser.in_trash() {
            segments.push((L10N.tr("trash"), None, "user-trash"));
        }
        if let Some(path) = self.shown_path().filter(|_| !self.browser.in_trash()) {
            let home = std::env::var_os("HOME").map(PathBuf::from);
            let start = home
                .as_deref()
                .filter(|home| path.starts_with(home))
                .map(Path::to_path_buf);
            let mut walked = PathBuf::new();
            for component in path.components() {
                walked.push(component);
                let is_start = start.as_deref() == Some(walked.as_path());
                if start
                    .as_ref()
                    .is_some_and(|start| !walked.starts_with(start))
                {
                    continue;
                }
                let name = walked.file_name().map_or_else(
                    || "/".to_owned(),
                    |name| name.to_string_lossy().into_owned(),
                );
                let icon = if is_start {
                    "user-home"
                } else if walked.is_dir() {
                    "folder"
                } else {
                    "text-x-generic"
                };
                segments.push((name, Some(walked.clone()), icon));
            }
        }
        let available = status_x - bar.x - 18.0;
        let mut widths = segments
            .iter()
            .map(|(name, _, _)| {
                let style = TextStyle::role(TextRole::Callout, muted);
                self.text()
                    .paragraph(name, style, None, TextAlign::Start)
                    .width()
                    + 38.0
            })
            .collect::<Vec<_>>();
        while widths.len() > 1 && widths.iter().sum::<f32>() > available {
            widths.remove(0);
            segments.remove(0);
        }
        let mut x = bar.x + 20.0;
        let mut row = Vec::new();
        let count = segments.len();
        for (index, (name, path, icon)) in segments.into_iter().enumerate() {
            let symbol = format!("{icon}-symbolic");
            if let Some(image) =
                self.icons
                    .symbol(&[&symbol, "folder-symbolic"], ICON_PIXELS, muted)
            {
                row.push(Item::Image {
                    rect: Rect::new(
                        x,
                        bar.y + ((PATH_BAR_HEIGHT - ICON_SIZE) / 2.0).round(),
                        ICON_SIZE,
                        ICON_SIZE,
                    ),
                    image,
                });
            }
            let paragraph = self.text().paragraph(
                &name,
                TextStyle::role(TextRole::Callout, muted),
                None,
                TextAlign::Start,
            );
            let width = paragraph.width();
            row.push(Item::Text {
                origin: Point::new(
                    x + 20.0,
                    bar.y + ((PATH_BAR_HEIGHT - paragraph.height()) / 2.0).round(),
                ),
                paragraph,
            });
            if let Some(path) = path {
                let area = Rect::new(x - 4.0, bar.y + 3.0, width + 28.0, PATH_BAR_HEIGHT - 6.0);
                let hit = Hit::Segment(path);
                if self.hover.as_ref() == Some(&hit) {
                    row.insert(0, self.fill(area, Radius::S, self.palette.well));
                }
                self.hits.push((area, hit));
            }
            x += width + 24.0;
            if index + 1 < count {
                row.push(self.line("\u{203A}", NAME_SIZE, muted, x, bar.y, PATH_BAR_HEIGHT));
                x += 14.0;
            }
        }
        items.push(Item::Clip {
            shape: Shape::rect(Rect::new(
                bar.x,
                bar.y,
                (status_x - bar.x - 8.0).max(0.0),
                bar.height,
            )),
            items: row,
        });
    }

    fn status_text(&self) -> String {
        let column = self.browser.active_column();
        let count = column.entries.len();
        let selected = column.marked.len();
        let items = if selected > 1 {
            L10N.tr_args(
                "selected-count",
                [("selected", selected.into()), ("count", count.into())],
            )
        } else {
            L10N.tr_args("item-count", [("count", count.into())])
        };
        let free = column
            .folder()
            .and_then(|folder| rustix::fs::statvfs(folder).ok())
            .map(|stats| stats.f_bavail * stats.f_frsize);
        match free {
            Some(bytes) => L10N.tr_args(
                "status-free",
                [("items", items.into()), ("free", format_size(bytes).into())],
            ),
            None => items,
        }
    }

    fn paint_progress(&mut self, items: &mut Vec<Item>) {
        let content = self.content_frame();
        let now = Instant::now();
        let mut y = content.y + content.height;
        let jobs = self
            .operations
            .running()
            .iter()
            .enumerate()
            .filter(|(_, running)| running.visible(now))
            .map(|(index, running)| {
                let progress = running.progress.clone();
                let total = progress.items_total.max(running.items as u64);
                let message = match running.purpose {
                    Purpose::Do(action) => format!("progress-{}", action_id(action)),
                    Purpose::Undo(_) => "progress-undo".to_owned(),
                    Purpose::Redo(_) => "progress-redo".to_owned(),
                    Purpose::Erase => "progress-erase".to_owned(),
                };
                let label = L10N.tr_args(
                    &message,
                    [
                        ("done", progress.items_done.min(total).into()),
                        ("total", total.into()),
                    ],
                );
                let fraction = if progress.bytes_total > 0 {
                    progress.bytes_done as f32 / progress.bytes_total as f32
                } else if total > 0 {
                    progress.items_done as f32 / total as f32
                } else {
                    0.0
                };
                let detail = (progress.bytes_total > 0).then(|| {
                    L10N.tr_args(
                        "progress-bytes",
                        [
                            ("done", format_size(progress.bytes_done).into()),
                            ("total", format_size(progress.bytes_total).into()),
                        ],
                    )
                });
                (index, label, detail, fraction.clamp(0.0, 1.0))
            })
            .collect::<Vec<_>>();
        for (index, label, detail, fraction) in jobs {
            let area = Rect::new(content.x, y, content.width, PROGRESS_HEIGHT);
            items.push(self.fill(area, 0.0, self.palette.sheet));
            items.push(self.fill(
                Rect::new(area.x, area.y, area.width, 1.0),
                0.0,
                self.palette.hairline,
            ));
            let primary = self.palette.ink;
            let muted = self.palette.ink2;
            items.push(self.line(
                &label,
                SMALL_SIZE + 1.0,
                primary,
                area.x + 14.0,
                y + 4.0,
                18.0,
            ));
            if let Some(detail) = detail {
                let paragraph = self.label(&detail, SMALL_SIZE, muted);
                items.push(Item::Text {
                    origin: Point::new(area.x + area.width - 44.0 - paragraph.width(), y + 6.0),
                    paragraph,
                });
            }
            let track = Rect::new(area.x + 14.0, y + 28.0, area.width - 58.0, 6.0);
            self.paint_progress_cells(track, fraction, items);
            let cancel = Rect::new(area.x + area.width - 34.0, y + 17.0, 22.0, 22.0);
            let hovered = self.hover == Some(Hit::CancelJob(index));
            items.push(self.fill(
                cancel,
                cancel.height / 2.0,
                match hovered {
                    true => self.palette.well.mix(self.palette.ink, 0.08),
                    false => self.palette.well,
                },
            ));
            let close = Rect::new(cancel.x + 3.0, cancel.y + 3.0, ICON_SIZE, ICON_SIZE);
            if let Some(image) =
                self.icons
                    .symbol(&["window-close-symbolic"], ICON_PIXELS, self.palette.ink2)
            {
                items.push(Item::Image { rect: close, image });
            }
            self.hits.push((cancel, Hit::CancelJob(index)));
            y += PROGRESS_HEIGHT;
        }
    }

    fn paint_progress_cells(&self, track: Rect, fraction: f32, items: &mut Vec<Item>) {
        let cells = PROGRESS_CELLS as f32;
        let cell = (track.width - PROGRESS_GAP * (cells - 1.0)) / cells;
        let lit = (fraction * cells).round() as usize;
        for index in 0..PROGRESS_CELLS {
            let x = track.x + index as f32 * (cell + PROGRESS_GAP);
            let color = match index < lit {
                true => self.palette.accent,
                false => self.palette.well,
            };
            items.push(self.fill(Rect::new(x, track.y, cell, track.height), 1.0, color));
        }
    }

    fn paint_inspector(&mut self, items: &mut Vec<Item>) {
        let sheet = self.sheet_frame();
        let area = Rect::new(
            sheet.x + sheet.width - INSPECTOR_WIDTH,
            sheet.y,
            INSPECTOR_WIDTH,
            sheet.height,
        );
        self.hits.push((area, Hit::Inspector));
        items.push(self.fill(area, 0.0, self.palette.sheet));
        items.push(self.fill(
            Rect::new(area.x, area.y, 1.0, area.height),
            0.0,
            self.palette.hairline,
        ));
        let Some(info) = self.info.take() else {
            return;
        };
        let mut content = Vec::new();
        let inner = area.inset(PADDING);
        let mut y = inner.y;
        let primary = self.palette.ink;
        let muted = self.palette.ink2;
        let preview_area = Rect::new(inner.x, y, inner.width, 150.0);
        let thumbnail = match info.entries.as_slice() {
            [entry] => self.thumbs.get(entry, ThumbnailSize::Large),
            _ => None,
        };
        match (&info.preview, info.entries.as_slice()) {
            (Some(Preview::Image(image)), [_]) => content.push(Item::Image {
                rect: fitted(preview_area, image),
                image: image.clone(),
            }),
            (Some(Preview::Other { .. }), [_]) if let Some(image) = thumbnail => {
                content.push(Item::Image {
                    rect: fitted(preview_area, &image),
                    image,
                });
            }
            (_, [entry]) => {
                if let Some(image) = self.entry_icon(entry, LARGE_ICON_PIXELS) {
                    content.push(Item::Image {
                        rect: centred(preview_area, 128.0, 128.0),
                        image,
                    });
                }
            }
            _ => {}
        }
        y += preview_area.height + 12.0;
        let title = match info.entries.as_slice() {
            [entry] => entry.name.clone(),
            [] => L10N.tr("no-selection"),
            entries => L10N.tr_args("item-count", [("count", entries.len().into())]),
        };
        let title = self.text().paragraph(
            &title,
            TextStyle::new(NAME_SIZE + 1.0, primary).weight(TextStyle::SEMIBOLD),
            Some(inner.width),
            TextAlign::Center,
        );
        let title_height = title.height();
        content.push(Item::Text {
            origin: Point::new(inner.x, y),
            paragraph: title,
        });
        y += title_height + 2.0;
        if !info.kind.is_empty() {
            let kind = self.text().paragraph(
                &info.kind,
                TextStyle::new(SMALL_SIZE, muted),
                Some(inner.width),
                TextAlign::Center,
            );
            let height = kind.height();
            content.push(Item::Text {
                origin: Point::new(inner.x, y),
                paragraph: kind,
            });
            y += height;
        }
        y += 14.0;
        let mut facts: Vec<(String, String)> = Vec::new();
        match info.entries.as_slice() {
            [entry] => {
                let size = if entry.is_folder() {
                    match info.folder_size {
                        Some((items, bytes)) => L10N.tr_args(
                            "folder-size",
                            [("size", format_size(bytes).into()), ("count", items.into())],
                        ),
                        None => L10N.tr("calculating"),
                    }
                } else {
                    format_size(entry.size)
                };
                facts.push((L10N.tr("info-size"), size));
                if let Some(modified) = entry.modified {
                    facts.push((L10N.tr("info-modified"), format_date(modified)));
                }
                let location = match &entry.trashed {
                    Some(item) => item
                        .original
                        .parent()
                        .map(|parent| parent.display().to_string())
                        .unwrap_or_default(),
                    None => entry
                        .path
                        .parent()
                        .map(|parent| parent.display().to_string())
                        .unwrap_or_default(),
                };
                let location_label = if entry.trashed.is_some() {
                    L10N.tr("info-original-location")
                } else {
                    L10N.tr("info-where")
                };
                facts.push((location_label, location));
                if let Some(item) = &entry.trashed
                    && let Some(deleted) = item.deleted
                {
                    facts.push((
                        L10N.tr("info-deleted"),
                        deleted.strftime("%-d %b %Y, %H:%M").to_string(),
                    ));
                }
                if let Ok(metadata) = std::fs::symlink_metadata(&entry.path) {
                    facts.push((
                        L10N.tr("info-permissions"),
                        permissions(metadata.permissions().mode()),
                    ));
                }
                if entry.link
                    && let Ok(target) = std::fs::read_link(&entry.path)
                {
                    facts.push((L10N.tr("info-link"), target.display().to_string()));
                }
            }
            entries if entries.len() > 1 => {
                let bytes = entries
                    .iter()
                    .filter(|entry| !entry.is_folder())
                    .map(|entry| entry.size)
                    .sum::<u64>();
                facts.push((L10N.tr("info-size"), format_size(bytes)));
            }
            _ => {}
        }
        for (label, value) in facts {
            let label = self.label(&label, SMALL_SIZE, muted);
            let value_paragraph = self.text().paragraph(
                &value,
                TextStyle::new(SMALL_SIZE, primary),
                Some(inner.width - 90.0),
                TextAlign::Start,
            );
            let height = value_paragraph.height().max(label.height());
            content.push(Item::Text {
                origin: Point::new(inner.x, y),
                paragraph: label,
            });
            content.push(Item::Text {
                origin: Point::new(inner.x + 90.0, y),
                paragraph: value_paragraph,
            });
            y += height + 6.0;
        }
        if !info.apps.is_empty() {
            y += 10.0;
            let heading = self.paragraph(
                &L10N.tr("open-with"),
                SMALL_SIZE,
                muted,
                TextStyle::SEMIBOLD,
            );
            content.push(Item::Text {
                origin: Point::new(inner.x, y),
                paragraph: heading,
            });
            y += 20.0;
            for (index, app) in info.apps.iter().enumerate() {
                let row = Rect::new(inner.x, y, inner.width, 24.0);
                let chosen = info.chosen == Some(index);
                if chosen {
                    content.push(self.fill(row, Radius::S, self.palette.accent_soft));
                }
                let name = if info.default.as_deref() == Some(app.id.as_str()) {
                    L10N.tr_args("default-app", [("app", app.name.clone().into())])
                } else {
                    app.name.clone()
                };
                content.push(self.line(&name, NAME_SIZE, primary, row.x + 8.0, y, row.height));
                self.hits.push((row, Hit::ChooseApp(index)));
                y += 24.0;
            }
            y += 8.0;
            let open = self.button(&L10N.tr("open"), Point::new(inner.x, y), true, &mut content);
            self.hits.push((open, Hit::OpenWithChosen));
            let chosen_is_default = info
                .chosen
                .and_then(|index| info.apps.get(index))
                .is_some_and(|app| info.default.as_deref() == Some(app.id.as_str()));
            if !chosen_is_default {
                let change = self.button(
                    &L10N.tr("use-for-all"),
                    Point::new(open.x + open.width + 8.0, y),
                    false,
                    &mut content,
                );
                self.hits.push((change, Hit::UseForAll));
            }
        }
        items.push(Item::Clip {
            shape: Shape::rect(area),
            items: content,
        });
        self.info = Some(info);
    }

    fn button(&mut self, title: &str, origin: Point, default: bool, items: &mut Vec<Item>) -> Rect {
        let (color, weight) = if default {
            (self.palette.on_accent, TextStyle::SEMIBOLD)
        } else {
            (self.palette.ink, TextStyle::MEDIUM)
        };
        let paragraph = self.paragraph(title, NAME_SIZE, color, weight);
        let rect = Rect::new(
            origin.x,
            origin.y,
            (paragraph.width() + 28.0).ceil(),
            BUTTON_HEIGHT,
        );
        let fill = if default {
            self.palette.accent
        } else {
            self.palette.well
        };
        items.push(self.fill(rect, Radius::S, fill));
        items.push(Item::Text {
            origin: Point::new(
                rect.x + 14.0,
                rect.y + ((BUTTON_HEIGHT - paragraph.height()) / 2.0).round(),
            ),
            paragraph,
        });
        rect
    }

    fn paint_notice(&mut self, items: &mut Vec<Item>) {
        let Some((text, _)) = self.notice.clone() else {
            return;
        };
        let content = self.content_frame();
        let chassis = self.palette.chassis;
        let paragraph = self.text().paragraph(
            &text,
            TextStyle::new(NAME_SIZE, chassis),
            Some(content.width - 80.0),
            TextAlign::Start,
        );
        let width = paragraph.width() + 32.0;
        let height = paragraph.height() + 16.0;
        let rect = Rect::new(
            content.x + ((content.width - width) / 2.0).round(),
            content.y + content.height - height - 16.0,
            width,
            height,
        );
        self.overlay_shadows(Shape::rounded(rect, Radius::L), items);
        items.push(self.fill(rect, Radius::L, self.palette.ink));
        items.push(Item::Text {
            origin: Point::new(rect.x + 16.0, rect.y + 8.0),
            paragraph,
        });
    }

    pub fn quick_look_frame(&self) -> Rect {
        let sheet = self.sheet_frame();
        centred(
            sheet,
            (sheet.width * QUICK_LOOK_SHARE).round(),
            (sheet.height * QUICK_LOOK_SHARE).round(),
        )
    }

    fn overlay_shadows(&self, shape: Shape, items: &mut Vec<Item>) {
        items.extend(
            Elevation::Overlay
                .shadows(self.palette.scheme, true)
                .into_iter()
                .map(|shadow| Item::Shadow {
                    shape,
                    offset: Point::new(0.0, shadow.offset),
                    blur_radius: shadow.blur,
                    color: Color::BLACK.with_opacity(shadow.opacity),
                }),
        );
    }

    fn panel(&self, panel: Rect, items: &mut Vec<Item>) -> Shape {
        let shape = Shape::rounded(panel, Radius::XL);
        self.overlay_shadows(shape, items);
        items.push(Item::Fill {
            shape,
            paint: self.palette.overlay.into(),
        });
        shape
    }

    fn paint_quick_look(&mut self, items: &mut Vec<Item>) {
        let Some(quick_look) = self.quick_look.take() else {
            return;
        };
        let panel = self.quick_look_frame();
        let shape = self.panel(panel, items);
        let primary = self.palette.ink;
        let title = self.text().paragraph(
            &quick_look.entry.name,
            TextStyle::new(NAME_SIZE, primary).weight(TextStyle::SEMIBOLD),
            Some(panel.width - 2.0 * PADDING),
            TextAlign::Center,
        );
        let title_y = panel.y + ((QUICK_LOOK_HEADER - title.height()) / 2.0).round();
        let mut content = vec![Item::Text {
            origin: Point::new(panel.x + PADDING, title_y),
            paragraph: title,
        }];
        let area = Rect::new(
            panel.x + PADDING,
            panel.y + QUICK_LOOK_HEADER,
            panel.width - 2.0 * PADDING,
            panel.height - QUICK_LOOK_HEADER - PADDING,
        );
        match &quick_look.preview {
            Some(Preview::Image(image)) => content.push(Item::Image {
                rect: fitted(area, image),
                image: image.clone(),
            }),
            Some(Preview::Text(text)) => {
                let lines = (area.height / CODE_SIZE).ceil() as usize + 1;
                let shown = text.lines().take(lines).collect::<Vec<_>>().join("\n");
                let paragraph = self.text().paragraph(
                    &shown,
                    TextStyle::new(CODE_SIZE, primary).monospace(),
                    None,
                    TextAlign::Start,
                );
                content.push(Item::Clip {
                    shape: Shape::rect(area),
                    items: vec![Item::Text {
                        origin: Point::new(area.x, area.y),
                        paragraph,
                    }],
                });
            }
            Some(Preview::Folder { items: count }) => {
                let summary = L10N.tr_args("item-count", [("count", (*count).into())]);
                self.summary(
                    &quick_look.entry,
                    area,
                    &L10N.tr("folder"),
                    &summary,
                    &mut content,
                );
            }
            Some(Preview::Other { .. })
                if let Some(image) = self.thumbs.get(&quick_look.entry, ThumbnailSize::XLarge) =>
            {
                content.push(Item::Image {
                    rect: fitted(area, &image),
                    image,
                });
            }
            Some(Preview::Other { size }) => {
                let mime = self.apps.mime_for(&quick_look.entry.path);
                let kind = self.apps.kind(&mime);
                self.summary(
                    &quick_look.entry,
                    area,
                    &kind,
                    &format_size(*size),
                    &mut content,
                );
            }
            None => {
                self.summary(&quick_look.entry, area, "", "", &mut content);
            }
        }
        items.push(Item::Clip {
            shape,
            items: content,
        });
        self.quick_look = Some(quick_look);
    }

    fn summary(
        &mut self,
        entry: &Entry,
        area: Rect,
        kind: &str,
        detail: &str,
        items: &mut Vec<Item>,
    ) {
        let icon = self.entry_icon(entry, LARGE_ICON_PIXELS);
        let primary = self.palette.ink;
        let muted = self.palette.ink2;
        let kind = self.text().paragraph(
            kind,
            TextStyle::new(NAME_SIZE + 4.0, primary).weight(TextStyle::MEDIUM),
            Some(area.width),
            TextAlign::Center,
        );
        let detail = self.text().paragraph(
            detail,
            TextStyle::new(NAME_SIZE, muted),
            Some(area.width),
            TextAlign::Center,
        );
        let icon_size = 128.0;
        let total = icon_size + 12.0 + kind.height() + 4.0 + detail.height();
        let top = area.y + ((area.height - total) / 2.0).round();
        if let Some(image) = icon {
            items.push(Item::Image {
                rect: Rect::new(
                    area.x + ((area.width - icon_size) / 2.0).round(),
                    top,
                    icon_size,
                    icon_size,
                ),
                image,
            });
        }
        let kind_y = top + icon_size + 12.0;
        let detail_y = kind_y + kind.height() + 4.0;
        items.push(Item::Text {
            origin: Point::new(area.x, kind_y),
            paragraph: kind,
        });
        items.push(Item::Text {
            origin: Point::new(area.x, detail_y),
            paragraph: detail,
        });
    }
}

fn action_id(action: unbound_fs::Action) -> &'static str {
    match action {
        unbound_fs::Action::Copy => "copy",
        unbound_fs::Action::Move => "move",
        unbound_fs::Action::Duplicate => "duplicate",
        unbound_fs::Action::Rename => "rename",
        unbound_fs::Action::NewFolder | unbound_fs::Action::NewDocument => "new-folder",
        unbound_fs::Action::Trash => "trash",
        unbound_fs::Action::PutBack => "put-back",
        unbound_fs::Action::Compress => "compress",
        unbound_fs::Action::Extract => "extract",
    }
}
