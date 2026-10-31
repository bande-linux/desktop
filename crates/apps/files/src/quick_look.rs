use unbound_fs::{Entry, format_size};
use unbound_preview::Preview;
use unbound_thumbnails::ThumbnailSize;
use unbound_toolkit::ui::{Key, Modifiers, Scroll, ScrollState};
use unbound_toolkit::{
    Image, Item, Paragraph, Point, Radius, Rect, Shape, TextAlign, TextRole, TextStyle,
};
use unbound_xdg::DesktopEntry;

use crate::L10N;
use crate::view::{FilesView, Hit, Request, contains};

const SHARE: f32 = 0.8;
const HEADER: f32 = 40.0;
const PADDING: f32 = 16.0;
const CODE_SIZE: f32 = 11.0;
const ICON: f32 = 16.0;
const ICON_PIXELS: u32 = 32;
const ZOOM_STEP: f32 = 1.25;
const MAX_ZOOM: f32 = 8.0;

pub struct QuickLook {
    pub entry: Entry,
    pub preview: Option<Preview<Image>>,
    pub items: Vec<Entry>,
    pub full_screen: bool,
    pub zoom: f32,
    pub app: Option<DesktopEntry>,
    pan: Point,
    scroll: ScrollState,
}

impl QuickLook {
    pub fn position(&self) -> Option<(usize, usize)> {
        let at = self
            .items
            .iter()
            .position(|item| item.path == self.entry.path)?;
        (self.items.len() > 1).then_some((at + 1, self.items.len()))
    }

    fn page(&self) -> Option<&str> {
        match &self.preview {
            Some(Preview::Text(text)) => Some(text),
            _ => None,
        }
    }

    fn picture(&self) -> Option<&Image> {
        match &self.preview {
            Some(Preview::Image(image)) => Some(image),
            _ => None,
        }
    }
}

fn zoomed(area: Rect, image: &Image, zoom: f32, pan: Point) -> Rect {
    let fit = crate::paint::fitted(area, image);
    let (width, height) = (fit.width * zoom, fit.height * zoom);
    Rect::new(
        (area.x + (area.width - width) / 2.0 + pan.x).round(),
        (area.y + (area.height - height) / 2.0 + pan.y).round(),
        width.round(),
        height.round(),
    )
}

fn clamped_pan(area: Rect, image: &Image, zoom: f32, pan: Point) -> Point {
    let fit = crate::paint::fitted(area, image);
    let room_x = ((fit.width * zoom - area.width) / 2.0).max(0.0);
    let room_y = ((fit.height * zoom - area.height) / 2.0).max(0.0);
    Point::new(pan.x.clamp(-room_x, room_x), pan.y.clamp(-room_y, room_y))
}

impl FilesView {
    fn quick_look_of(&self, entry: Entry, items: Vec<Entry>, full_screen: bool) -> QuickLook {
        let app = match entry.is_folder() || entry.trashed.is_some() {
            true => None,
            false => self.apps.default_for_path(&entry.path).cloned(),
        };
        QuickLook {
            entry,
            preview: None,
            items,
            full_screen,
            zoom: 1.0,
            app,
            pan: Point::default(),
            scroll: ScrollState::default(),
        }
    }

    fn show_quick_look(&mut self, entry: Entry, items: Vec<Entry>, full_screen: bool) {
        self.loader.preview(entry.clone());
        self.quick_look = Some(self.quick_look_of(entry, items, full_screen));
    }

    pub(crate) fn toggle_quick_look(&mut self) {
        if self.quick_look.take().is_none() {
            self.open_quick_look(false);
        }
    }

    pub(crate) fn open_quick_look(&mut self, full_screen: bool) {
        let selection = self
            .browser
            .selection()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let Some(entry) = self
            .browser
            .selected()
            .cloned()
            .or_else(|| selection.first().cloned())
        else {
            return;
        };
        let items = match selection.len() > 1 {
            true => selection,
            false => Vec::new(),
        };
        self.show_quick_look(entry, items, full_screen);
    }

    pub(crate) fn follow_selection_in_quick_look(&mut self, selected: Option<&Entry>) {
        let Some(shown) = &self.quick_look else {
            return;
        };
        if !shown.items.is_empty() || selected.map(|entry| &entry.path) == Some(&shown.entry.path) {
            return;
        }
        let full_screen = shown.full_screen;
        match selected {
            Some(entry) => {
                self.quick_look = Some(self.quick_look_of(entry.clone(), Vec::new(), full_screen));
            }
            None => self.quick_look = None,
        }
    }

    fn step_quick_look(&mut self, forward: bool) -> bool {
        let Some(shown) = &self.quick_look else {
            return false;
        };
        let Some((at, count)) = shown.position() else {
            return false;
        };
        let next = match forward {
            true => at.min(count - 1),
            false => at.saturating_sub(2),
        };
        if next + 1 == at {
            return true;
        }
        let (items, full_screen) = (shown.items.clone(), shown.full_screen);
        if let Some(entry) = items.get(next).cloned() {
            self.show_quick_look(entry, items, full_screen);
        }
        true
    }

    pub(crate) fn toggle_quick_look_full_screen(&mut self) {
        match &mut self.quick_look {
            Some(shown) => shown.full_screen = !shown.full_screen,
            None => self.open_quick_look(true),
        }
    }

    fn zoom_quick_look(&mut self, bigger: bool) -> bool {
        let Some(shown) = &mut self.quick_look else {
            return false;
        };
        if shown.picture().is_none() {
            return false;
        }
        shown.zoom = match bigger {
            true => (shown.zoom * ZOOM_STEP).min(MAX_ZOOM),
            false => (shown.zoom / ZOOM_STEP).max(1.0),
        };
        true
    }

    fn page_quick_look(&mut self, down: bool) -> bool {
        let area = self.quick_look_area();
        let Some(shown) = &mut self.quick_look else {
            return false;
        };
        if shown.page().is_none() {
            return false;
        }
        let cx = self.ui.context();
        let page = (area.height - 2.0 * CODE_SIZE).max(CODE_SIZE);
        let target = shown.scroll.offset(cx) + if down { page } else { -page };
        shown.scroll.scroll_to(cx, target);
        true
    }

    pub(crate) fn quick_look_key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        let command = modifiers.logo;
        let plain = !command && !modifiers.shift && !modifiers.alt && !modifiers.control;
        match key {
            Key::Escape => match &mut self.quick_look {
                Some(shown) if shown.full_screen => shown.full_screen = false,
                _ => self.quick_look = None,
            },
            Key::Left | Key::Right if plain => return self.step_quick_look(key == Key::Right),
            Key::PageDown | Key::PageUp if plain => {
                return self.page_quick_look(key == Key::PageDown);
            }
            Key::Character('y' | 'Y') if command && modifiers.alt => {
                self.toggle_quick_look_full_screen();
            }
            Key::Character('=' | '+') if command => return self.zoom_quick_look(true),
            Key::Character('-') if command => return self.zoom_quick_look(false),
            _ => return false,
        }
        true
    }

    pub(crate) fn quick_look_scroll(&mut self, scroll: &Scroll) -> bool {
        if self.quick_look.is_none() || !contains(self.quick_look_frame(), scroll.position) {
            return false;
        }
        let area = self.quick_look_area();
        let cx = self.ui.context();
        if let Some(shown) = &mut self.quick_look {
            if shown.page().is_some() {
                shown.scroll.scroll(cx, scroll);
            } else if let Some(image) = shown.picture()
                && shown.zoom > 1.0
            {
                let moved = Point::new(shown.pan.x - scroll.delta.x, shown.pan.y - scroll.delta.y);
                shown.pan = clamped_pan(area, image, shown.zoom, moved);
            }
        }
        true
    }

    pub(crate) fn open_quick_look_item(&mut self) {
        let Some(shown) = &self.quick_look else {
            return;
        };
        if let Some(app) = &shown.app {
            self.requests.push(Request::Open {
                app: Box::new(app.clone()),
                files: vec![shown.entry.path.clone()],
            });
        }
    }

    pub fn quick_look_frame(&self) -> Rect {
        if self
            .quick_look
            .as_ref()
            .is_some_and(|shown| shown.full_screen)
        {
            return Rect::new(0.0, 0.0, self.size.0, self.size.1);
        }
        let sheet = self.sheet_frame();
        let (width, height) = (
            (sheet.width * SHARE).round(),
            (sheet.height * SHARE).round(),
        );
        Rect::new(
            sheet.x + ((sheet.width - width) / 2.0).round(),
            sheet.y + ((sheet.height - height) / 2.0).round(),
            width,
            height,
        )
    }

    fn quick_look_area(&self) -> Rect {
        let panel = self.quick_look_frame();
        Rect::new(
            panel.x + PADDING,
            panel.y + HEADER,
            (panel.width - 2.0 * PADDING).max(0.0),
            (panel.height - HEADER - PADDING).max(0.0),
        )
    }

    pub fn quick_look_scrolled(&mut self) -> f32 {
        let cx = self.ui.context();
        self.quick_look
            .as_ref()
            .map_or(0.0, |shown| shown.scroll.offset(cx))
    }

    fn header_text(&mut self, text: &str, style: TextStyle, width: Option<f32>) -> Paragraph {
        self.text().paragraph(text, style, width, TextAlign::Start)
    }

    fn paint_quick_look_header(&mut self, shown: &QuickLook, panel: Rect, items: &mut Vec<Item>) {
        let palette = self.palette;
        let middle = |height: f32| panel.y + ((HEADER - height) / 2.0).round();
        let toggle = Rect::new(panel.x + 10.0, middle(24.0), 24.0, 24.0);
        if self.hover == Some(Hit::QuickLookFullScreen) {
            items.push(Item::Fill {
                shape: Shape::rounded(toggle, Radius::S),
                paint: palette.well.into(),
            });
        }
        let symbols: &[&str] = match shown.full_screen {
            true => &["view-restore-symbolic", "window-restore-symbolic"],
            false => &["view-fullscreen-symbolic", "window-maximize-symbolic"],
        };
        if let Some(image) = symbols
            .iter()
            .find_map(|name| self.icons.symbol(&[name], ICON_PIXELS, palette.ink2))
        {
            items.push(Item::Image {
                rect: Rect::new(toggle.x + 4.0, toggle.y + 4.0, ICON, ICON),
                image,
            });
        }
        self.hits.push((toggle, Hit::QuickLookFullScreen));
        let mut left = toggle.x + toggle.width + 10.0;
        if let Some((at, count)) = shown.position() {
            let counter = self.header_text(
                &L10N.tr_args(
                    "quick-look-position",
                    [("at", at.into()), ("count", count.into())],
                ),
                TextStyle::role(TextRole::Callout, palette.ink2).tabular(),
                None,
            );
            let width = counter.width();
            items.push(Item::Text {
                origin: Point::new(left, middle(counter.height())),
                paragraph: counter,
            });
            left += width + 10.0;
        }
        let mut right = panel.x + panel.width - 10.0;
        if let Some(app) = &shown.app {
            let title = L10N.tr_args("quick-look-open-with", [("app", app.name.clone().into())]);
            let label = self.header_text(
                &title,
                TextStyle::role(TextRole::Body, palette.ink).weight(TextStyle::MEDIUM),
                None,
            );
            let button = Rect::new(
                right - (label.width() + 24.0).ceil(),
                middle(24.0),
                (label.width() + 24.0).ceil(),
                24.0,
            );
            let fill = match self.hover == Some(Hit::QuickLookOpen) {
                true => palette.well.mix(palette.ink, 0.08),
                false => palette.well,
            };
            items.push(Item::Fill {
                shape: Shape::rounded(button, Radius::S),
                paint: fill.into(),
            });
            items.push(Item::Text {
                origin: Point::new(button.x + 12.0, middle(label.height())),
                paragraph: label,
            });
            self.hits.push((button, Hit::QuickLookOpen));
            right = button.x - 10.0;
        }
        let room = (2.0 * (right - panel.x - panel.width / 2.0))
            .min(2.0 * (panel.x + panel.width / 2.0 - left))
            .max(0.0);
        let title = self.text().paragraph(
            &shown.entry.name,
            TextStyle::role(TextRole::BodyStrong, palette.ink),
            Some(room),
            TextAlign::Center,
        );
        let title_y = middle(title.height());
        items.push(Item::Clip {
            shape: Shape::rect(Rect::new(
                panel.x + (panel.width - room) / 2.0,
                panel.y,
                room,
                HEADER,
            )),
            items: vec![Item::Text {
                origin: Point::new(panel.x + (panel.width - room) / 2.0, title_y),
                paragraph: title,
            }],
        });
    }

    fn paint_quick_look_page(&mut self, shown: &mut QuickLook, area: Rect, items: &mut Vec<Item>) {
        let Some(text) = shown.page().map(str::to_owned) else {
            return;
        };
        let style = TextStyle::new(CODE_SIZE, self.palette.ink).monospace();
        let line = self
            .text()
            .paragraph("M", style, None, TextAlign::Start)
            .height()
            .max(1.0);
        let count = text.lines().count();
        shown.scroll.set_extent(count as f32 * line, area.height);
        let cx = self.ui.context();
        let offset = shown.scroll.offset(cx);
        let first = (offset.max(0.0) / line).floor() as usize;
        let fitting = (area.height / line).ceil() as usize + 2;
        let visible = text
            .lines()
            .skip(first)
            .take(fitting)
            .collect::<Vec<_>>()
            .join("\n");
        let paragraph = self
            .text()
            .paragraph(&visible, style, None, TextAlign::Start);
        items.push(Item::Clip {
            shape: Shape::rect(area),
            items: vec![Item::Text {
                origin: Point::new(area.x, area.y + first as f32 * line - offset),
                paragraph,
            }],
        });
        let cx = self.ui.context();
        shown.scroll.paint_scrollbar(cx, area, items);
    }

    pub(crate) fn paint_quick_look(&mut self, items: &mut Vec<Item>) {
        let Some(mut shown) = self.quick_look.take() else {
            return;
        };
        let panel = self.quick_look_frame();
        let shape = match shown.full_screen {
            true => {
                let shape = Shape::rect(panel);
                items.push(Item::Fill {
                    shape,
                    paint: self.palette.overlay.into(),
                });
                shape
            }
            false => self.panel(panel, items),
        };
        self.hits.push((panel, Hit::QuickLook));
        let mut content = Vec::new();
        self.paint_quick_look_header(&shown, panel, &mut content);
        let area = self.quick_look_area();
        match &shown.preview {
            Some(Preview::Image(image)) => {
                shown.pan = clamped_pan(area, image, shown.zoom, shown.pan);
                content.push(Item::Clip {
                    shape: Shape::rect(area),
                    items: vec![Item::Image {
                        rect: zoomed(area, image, shown.zoom, shown.pan),
                        image: image.clone(),
                    }],
                });
            }
            Some(Preview::Text(_)) => self.paint_quick_look_page(&mut shown, area, &mut content),
            Some(Preview::Folder { items: count }) => {
                let summary = L10N.tr_args("item-count", [("count", (*count).into())]);
                self.summary(
                    &shown.entry,
                    area,
                    &L10N.tr("folder"),
                    &summary,
                    &mut content,
                );
            }
            Some(Preview::Other { .. })
                if let Some(image) = self.thumbs.get(&shown.entry, ThumbnailSize::XLarge) =>
            {
                content.push(Item::Image {
                    rect: crate::paint::fitted(area, &image),
                    image,
                });
            }
            Some(Preview::Other { size }) => {
                let mime = self.apps.mime_for(&shown.entry.path);
                let kind = self.apps.kind(&mime);
                self.summary(&shown.entry, area, &kind, &format_size(*size), &mut content);
            }
            None => self.summary(&shown.entry, area, "", "", &mut content),
        }
        items.push(Item::Clip {
            shape,
            items: content,
        });
        self.quick_look = Some(shown);
    }
}
