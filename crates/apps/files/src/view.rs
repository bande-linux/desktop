use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{
    Color, Item, Palette, Paragraph, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem,
};

use crate::browser::{Browser, Column};
use crate::listing::{Entry, Kind, format_size};
use crate::preview::{Preview, preview};

pub const COLUMN_WIDTH: f32 = 240.0;
pub const ROW_HEIGHT: f32 = 24.0;
const TOP_INSET: f32 = 6.0;
const ROW_INSET: f32 = 6.0;
const NAME_SIZE: f32 = 13.0;
const CODE_SIZE: f32 = 11.0;
const QUICK_LOOK_SHARE: f32 = 0.8;
const QUICK_LOOK_HEADER: f32 = 36.0;
const QUICK_LOOK_RADIUS: f32 = 12.0;
const QUICK_LOOK_PADDING: f32 = 16.0;

pub struct QuickLook {
    pub entry: Entry,
    pub preview: Preview,
}

pub struct FilesView {
    browser: Browser,
    quick_look: Option<QuickLook>,
    palette: Palette,
    text: TextSystem,
    size: (f32, f32),
}

impl FilesView {
    pub fn new(browser: Browser, palette: Palette, text: TextSystem) -> Self {
        Self {
            browser,
            quick_look: None,
            palette,
            text,
            size: (0.0, 0.0),
        }
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    pub fn browser(&self) -> &Browser {
        &self.browser
    }

    pub fn quick_look(&self) -> Option<&QuickLook> {
        self.quick_look.as_ref()
    }

    pub fn title(&self) -> String {
        let folder = self.browser.current_folder();
        folder.file_name().map_or_else(
            || folder.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }

    fn follow_selection(&mut self) {
        if self.quick_look.is_some() {
            self.quick_look = self.browser.selected().map(|entry| QuickLook {
                entry: entry.clone(),
                preview: preview(entry),
            });
        }
    }

    fn toggle_quick_look(&mut self) {
        self.quick_look = match self.quick_look.take() {
            Some(_) => None,
            None => self.browser.selected().map(|entry| QuickLook {
                entry: entry.clone(),
                preview: preview(entry),
            }),
        };
    }

    fn first_visible_column(&self) -> usize {
        let columns = self.browser.columns().len();
        let last = (self.browser.active() + 1).min(columns - 1);
        let fit = ((self.size.0 / COLUMN_WIDTH).floor() as usize).max(1);
        (last + 1).saturating_sub(fit)
    }

    fn rows_that_fit(&self) -> usize {
        (((self.size.1 - TOP_INSET) / ROW_HEIGHT).floor() as usize).max(1)
    }

    fn first_visible_row(&self, column: &Column) -> usize {
        column
            .selected
            .map_or(0, |row| (row + 1).saturating_sub(self.rows_that_fit()))
    }

    fn click(&mut self, at: Point) -> bool {
        let column = self.first_visible_column() + (at.x / COLUMN_WIDTH).floor() as usize;
        let Some(shown) = self.browser.columns().get(column) else {
            return false;
        };
        if at.y < TOP_INSET {
            return false;
        }
        let row = self.first_visible_row(shown) + ((at.y - TOP_INSET) / ROW_HEIGHT) as usize;
        if row >= shown.entries.len() {
            return false;
        }
        self.browser.select(column, row);
        true
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        let changed = match event {
            Event::Key { key, modifiers } => self.key(*key, *modifiers),
            Event::PointerDown(at) => self.click(*at),
            _ => false,
        };
        if changed {
            self.follow_selection();
        }
        changed
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        let command = modifiers.logo;
        match key {
            Key::Space if !command => {
                self.toggle_quick_look();
                return true;
            }
            Key::Escape if self.quick_look.is_some() => {
                self.quick_look = None;
                return true;
            }
            Key::Character('.' | '>') if command && modifiers.shift => {
                self.browser.toggle_hidden();
            }
            Key::Up if command => return self.browser.enclosing_folder(),
            Key::Down if command => return self.browser.right(),
            Key::Down => self.browser.down(),
            Key::Up => self.browser.up(),
            Key::Right => return self.browser.right(),
            Key::Left => return self.browser.left(),
            _ => return false,
        }
        true
    }

    fn label(&mut self, text: &str, style: TextStyle, width: Option<f32>) -> Paragraph {
        self.text.paragraph(text, style, width, TextAlign::Start)
    }

    fn paint_column(&mut self, index: usize, x: f32, items: &mut Vec<Item>) {
        let column = self.browser.columns()[index].clone();
        let active = index == self.browser.active();
        let height = self.size.1;
        let frame = Rect::new(x, 0.0, COLUMN_WIDTH, height);
        let mut rows = Vec::new();
        let first = self.first_visible_row(&column);
        for (position, entry) in column.entries.iter().enumerate().skip(first) {
            let y = TOP_INSET + (position - first) as f32 * ROW_HEIGHT;
            if y > height {
                break;
            }
            let row = Rect::new(x + ROW_INSET, y, COLUMN_WIDTH - 2.0 * ROW_INSET, ROW_HEIGHT);
            let selected = column.selected == Some(position);
            let color = match (selected, active) {
                (true, true) => Color::hex(0xFFFFFF),
                _ => self.palette.label_primary,
            };
            if selected {
                rows.push(Item::Fill {
                    shape: Shape::rounded(row, 5.0),
                    paint: if active {
                        self.palette.accent_color.into()
                    } else {
                        self.palette.label_muted.with_alpha(56).into()
                    },
                });
            }
            let name = self.label(&entry.name, TextStyle::new(NAME_SIZE, color), None);
            let baseline = y + ((ROW_HEIGHT - name.height()) / 2.0).round();
            rows.push(Item::Text {
                origin: Point::new(row.x + 8.0, baseline),
                paragraph: name,
            });
            if entry.kind == Kind::Folder {
                let chevron = self.label(
                    "\u{203A}",
                    TextStyle::new(NAME_SIZE + 3.0, color.with_alpha(180)),
                    None,
                );
                let chevron_y = y + ((ROW_HEIGHT - chevron.height()) / 2.0).round();
                rows.push(Item::Text {
                    origin: Point::new(row.x + row.width - 8.0 - chevron.width(), chevron_y),
                    paragraph: chevron,
                });
            }
        }
        items.push(Item::Clip {
            shape: Shape::rect(frame),
            items: rows,
        });
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(x + COLUMN_WIDTH - 1.0, 0.0, 1.0, height)),
            paint: self.palette.separator_color.into(),
        });
    }

    pub fn quick_look_frame(&self) -> Rect {
        let (width, height) = self.size;
        let (panel_width, panel_height) = (
            (width * QUICK_LOOK_SHARE).round(),
            (height * QUICK_LOOK_SHARE).round(),
        );
        Rect::new(
            ((width - panel_width) / 2.0).round(),
            ((height - panel_height) / 2.0).round(),
            panel_width,
            panel_height,
        )
    }

    fn paint_quick_look(&mut self, items: &mut Vec<Item>) {
        let Some(QuickLook { entry, preview }) = self.quick_look.take() else {
            return;
        };
        let panel = self.quick_look_frame();
        let shape = Shape::rounded(panel, QUICK_LOOK_RADIUS);
        items.push(Item::Shadow {
            shape,
            offset: Point::new(0.0, 8.0),
            blur_radius: 24.0,
            color: Color::hex(0x000000).with_alpha(90),
        });
        items.push(Item::Fill {
            shape,
            paint: self.palette.surface_elevated.into(),
        });
        let title = self.text.paragraph(
            &entry.name,
            TextStyle::new(NAME_SIZE, self.palette.label_primary).weight(TextStyle::SEMIBOLD),
            Some(panel.width - 2.0 * QUICK_LOOK_PADDING),
            TextAlign::Center,
        );
        let title_y = panel.y + ((QUICK_LOOK_HEADER - title.height()) / 2.0).round();
        let mut content = Vec::new();
        content.push(Item::Text {
            origin: Point::new(panel.x + QUICK_LOOK_PADDING, title_y),
            paragraph: title,
        });
        let area = Rect::new(
            panel.x + QUICK_LOOK_PADDING,
            panel.y + QUICK_LOOK_HEADER,
            panel.width - 2.0 * QUICK_LOOK_PADDING,
            panel.height - QUICK_LOOK_HEADER - QUICK_LOOK_PADDING,
        );
        match &preview {
            Preview::Image(image) => {
                let (width, height) = (image.width() as f32, image.height() as f32);
                let scale = (area.width / width).min(area.height / height).min(1.0);
                let (shown_width, shown_height) =
                    ((width * scale).round(), (height * scale).round());
                content.push(Item::Image {
                    rect: Rect::new(
                        area.x + ((area.width - shown_width) / 2.0).round(),
                        area.y + ((area.height - shown_height) / 2.0).round(),
                        shown_width,
                        shown_height,
                    ),
                    image: image.clone(),
                });
            }
            Preview::Text(text) => {
                let lines = (area.height / CODE_SIZE).ceil() as usize + 1;
                let shown = text.lines().take(lines).collect::<Vec<_>>().join("\n");
                let paragraph = self.label(
                    &shown,
                    TextStyle::new(CODE_SIZE, self.palette.label_primary).monospace(),
                    None,
                );
                content.push(Item::Clip {
                    shape: Shape::rect(area),
                    items: vec![Item::Text {
                        origin: Point::new(area.x, area.y),
                        paragraph,
                    }],
                });
            }
            Preview::Folder { items: count } => {
                let summary = match count {
                    1 => "1 item".to_owned(),
                    count => format!("{count} items"),
                };
                self.centred_summary(area, "Folder", &summary, &mut content);
            }
            Preview::Other { size } => {
                self.centred_summary(area, "Document", &format_size(*size), &mut content);
            }
        }
        items.push(Item::Clip {
            shape,
            items: content,
        });
        self.quick_look = Some(QuickLook { entry, preview });
    }

    fn centred_summary(&mut self, area: Rect, kind: &str, detail: &str, items: &mut Vec<Item>) {
        let kind = self.text.paragraph(
            kind,
            TextStyle::new(NAME_SIZE + 4.0, self.palette.label_primary).weight(TextStyle::MEDIUM),
            Some(area.width),
            TextAlign::Center,
        );
        let detail = self.text.paragraph(
            detail,
            TextStyle::new(NAME_SIZE, self.palette.label_muted),
            Some(area.width),
            TextAlign::Center,
        );
        let top = area.y + ((area.height - kind.height() - detail.height() - 4.0) / 2.0).round();
        let below = top + kind.height() + 4.0;
        items.push(Item::Text {
            origin: Point::new(area.x, top),
            paragraph: kind,
        });
        items.push(Item::Text {
            origin: Point::new(area.x, below),
            paragraph: detail,
        });
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        self.size = (width as f32, height as f32);
        let mut items = Vec::new();
        let first = self.first_visible_column();
        for index in first..self.browser.columns().len() {
            let x = (index - first) as f32 * COLUMN_WIDTH;
            if x >= self.size.0 {
                break;
            }
            self.paint_column(index, x, &mut items);
        }
        self.paint_quick_look(&mut items);
        Scene {
            width,
            height,
            background: self.palette.surface_background,
            items,
        }
    }
}
