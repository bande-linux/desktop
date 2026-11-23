use bande_l10n::{tr, trn};

use crate::settings::Density;
use crate::store::{AccountId, Filter, MessageId, Summary};
use crate::ui::format::list_date;
use crate::ui::reader::{Clock, Target};
use crate::ui::reader_pane::{Look, paint_link_target};
use crate::ui::sidebar::Place;
use crate::ui::sidebar::Row;
use crate::ui::symbols::Symbol;
use crate::ui::toolbar::{LIST_HEADER_HEIGHT, LIST_TITLE_HEIGHT, SIDEBAR_FOOT_HEIGHT};
use crate::ui::view::{Hit, MailView, Pane, Panes, Quick, READER_MAX_WIDTH, TOOLBAR_HEIGHT};
use bande_toolkit::Color;
use bande_toolkit::ui::{BUTTON_HEIGHT, Key, Scroll, ScrollPhase, ScrollSource, Size};
use bande_toolkit::{
    Elevation, Item, Paragraph, Point, Radius, Rect, Scene, Shape, TextAlign, TextRole, TextStyle,
};
use jiff::Timestamp;

const FLAG_COLORS: [u32; 7] = [
    0xFF3B30, 0xFF9500, 0xFFCC00, 0x34C759, 0x007AFF, 0xAF52DE, 0x8E8E93,
];

const CHILD_INDENT: f32 = 14.0;
const DRAG_LABEL_WIDTH: f32 = 240.0;
const SIDEBAR_INSET: f32 = 8.0;
const SIDEBAR_ROW: f32 = 28.0;
const SIDEBAR_HEADING: f32 = 30.0;
const SIDEBAR_TEXT_X: f32 = 18.0;
const SIDEBAR_INDENT: f32 = 14.0;
const ICON_SIZE: f32 = 16.0;
const ICON_PIXELS: u32 = 32;
const ICON_GAP: f32 = 8.0;
const LIST_PADDING: f32 = 10.0;
const LIST_TEXT_X: f32 = 26.0;
const LIST_TRAILING: f32 = 14.0;
const DOT: f32 = 8.0;
const ROW_INSET: f32 = 6.0;
const BADGE_GAP: f32 = 10.0;
const SMALL_SYMBOL: f32 = 13.0;
const LIST_HEADER_INSET: f32 = 14.0;
const ACCOUNT_MARK: f32 = 3.0;
const ACCOUNT_MARK_INSET: f32 = 2.0;
const ACCOUNT_COLORS: [usize; 5] = [4, 3, 5, 1, 0];
const GROUP_HEIGHT: f32 = 26.0;
const EMPTY_SYMBOL: f32 = 32.0;
const QUICK_SIZE: f32 = 22.0;
const QUICK_GAP: f32 = 2.0;
const SWIPE_COMMIT: f32 = 96.0;
const OPEN_FILE_WIDTH: f32 = 240.0;
const OPEN_FILE_BOTTOM: f32 = 64.0;

impl MailView {
    fn scale(&mut self) -> f32 {
        self.ui.context().text.scale()
    }

    pub(crate) fn sheet_frame(&self) -> Rect {
        let x = match self.panes() {
            Panes::Three => self.sidebar_size(),
            Panes::Two | Panes::One => self.sheet_inset,
        };
        Rect::new(
            x,
            TOOLBAR_HEIGHT,
            (self.size.0 - x - self.sheet_inset).max(0.0),
            (self.size.1 - TOOLBAR_HEIGHT - self.sheet_inset).max(0.0),
        )
    }

    fn sidebar_width(&self) -> f32 {
        match self.panes() == Panes::Three || self.overlay {
            true => self.sidebar_size(),
            false => 0.0,
        }
    }

    pub(crate) fn sidebar_frame(&self) -> Rect {
        Rect::new(
            0.0,
            TOOLBAR_HEIGHT,
            self.sidebar_width(),
            (self.size.1 - TOOLBAR_HEIGHT - SIDEBAR_FOOT_HEIGHT).max(0.0),
        )
    }

    pub(crate) fn sidebar_foot_frame(&self) -> Rect {
        Rect::new(
            0.0,
            (self.size.1 - SIDEBAR_FOOT_HEIGHT).max(TOOLBAR_HEIGHT),
            self.sidebar_width(),
            SIDEBAR_FOOT_HEIGHT,
        )
    }

    pub(crate) fn list_header_frame(&self) -> Rect {
        let sheet = self.sheet_frame();
        let width = match (self.panes(), self.single_reader) {
            (Panes::One, true) => 0.0,
            (Panes::One, false) => sheet.width,
            _ => self.list_size().min(sheet.width),
        };
        Rect::new(
            sheet.x,
            sheet.y,
            width,
            LIST_HEADER_HEIGHT.min(sheet.height),
        )
    }

    pub(crate) fn list_frame(&self) -> Rect {
        let header = self.list_header_frame();
        let sheet = self.sheet_frame();
        Rect::new(
            header.x,
            header.y + header.height,
            header.width,
            (sheet.height - header.height).max(0.0),
        )
    }

    pub(crate) fn reader_area(&self) -> Rect {
        let sheet = self.sheet_frame();
        let list = self.list_header_frame();
        let divider = match list.width > 0.0 {
            true => 1.0,
            false => 0.0,
        };
        let width = match (self.panes(), self.single_reader) {
            (Panes::One, false) => 0.0,
            _ => (sheet.width - list.width - divider).max(0.0),
        };
        Rect::new(list.x + list.width + divider, sheet.y, width, sheet.height)
    }

    pub(crate) fn reader_frame(&self) -> Rect {
        let area = self.reader_area();
        let width = area.width.min(READER_MAX_WIDTH);
        Rect::new(
            area.x + ((area.width - width) / 2.0).round(),
            area.y,
            width,
            area.height,
        )
    }

    pub(crate) fn row_top(&self, index: usize, row_height: f32) -> f32 {
        let groups = self.groups.partition_point(|(start, _)| *start <= index);
        index as f32 * row_height + groups as f32 * GROUP_HEIGHT
    }

    fn first_row_after(&self, offset: f32, row_height: f32) -> usize {
        let (mut low, mut high) = (0, self.messages.len());
        while low < high {
            let middle = low + (high - low) / 2;
            match self.row_top(middle, row_height) + row_height <= offset {
                true => low = middle + 1,
                false => high = middle,
            }
        }
        low
    }

    pub(crate) fn row_height(&mut self) -> f32 {
        let scale = self.scale();
        let lines = match self.settings.density {
            Density::Comfortable => 18.0 + 18.0 + 16.0 * 2.0,
            Density::Compact => 18.0 + 16.0,
        };
        (2.0 * LIST_PADDING + lines * scale).round()
    }

    fn sidebar_row_height(&mut self) -> f32 {
        (SIDEBAR_ROW * self.scale()).round()
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        self.size = (width as f32, height as f32);
        self.hits.clear();
        self.layout_toolbar();
        if let Some(guide) = self.setup.as_ref().filter(|_| self.welcoming()) {
            let area = Rect::new(
                0.0,
                TOOLBAR_HEIGHT,
                self.size.0,
                (self.size.1 - TOOLBAR_HEIGHT).max(0.0),
            );
            guide.place(&mut self.ui, area);
        }
        if let Some(open) = self.open_file.filter(|_| self.welcoming()) {
            let (width, height) = self.size;
            if let Some(host) = self.host_mut() {
                host.set_frame(
                    open,
                    Rect::new(
                        ((width - OPEN_FILE_WIDTH) / 2.0).round(),
                        height - OPEN_FILE_BOTTOM,
                        OPEN_FILE_WIDTH,
                        BUTTON_HEIGHT,
                    ),
                );
            }
        }
        self.ui.layout(Size::new(self.size.0, self.size.1));
        let (controls, sheet) = self.ui.paint_layers(width, height);
        let mut items = Vec::new();
        if self.welcoming() {
            items.extend(controls);
            items.extend(sheet);
            return Scene {
                width,
                height,
                background: self.palette.sheet,
                items,
            };
        }
        let overlay = self.overlay && self.panes() != Panes::Three;
        if !overlay {
            self.paint_sidebar(&mut items);
        }
        let sheet_frame = self.sheet_frame();
        let radius = match self.sheet_inset > 0.0 {
            true => Radius::concentric(Radius::XL, self.sheet_inset),
            false => 0.0,
        };
        let mut content = Vec::new();
        self.paint_list_header(&mut content);
        self.paint_list(&mut content);
        self.paint_reader(&mut content);
        items.push(Item::Clip {
            shape: Shape::rounded(sheet_frame, radius),
            items: content,
        });
        if overlay {
            let frame = self.sidebar_frame();
            let panel = Rect::new(
                frame.x,
                frame.y,
                frame.width,
                frame.height + SIDEBAR_FOOT_HEIGHT,
            );
            items.extend(Item::shadows(
                Shape::rect(panel),
                Elevation::Overlay,
                self.palette.scheme,
                true,
            ));
            items.push(Item::Fill {
                shape: Shape::rect(panel),
                paint: self.palette.chassis.into(),
            });
            self.paint_sidebar(&mut items);
        }
        self.paint_drag(&mut items);
        items.extend(controls);
        items.extend(sheet);
        Scene {
            width,
            height,
            background: self.palette.sheet,
            items,
        }
    }

    fn paint_drag(&mut self, items: &mut Vec<Item>) {
        let Some(drag) = self.drag.filter(|drag| drag.moving) else {
            return;
        };
        let chosen = self.chosen();
        let label = match chosen.as_slice() {
            [one] if !one.subject.is_empty() => one.subject.clone(),
            [_] => tr!("No Subject"),
            many => tr!("{count} messages", count = many.len()),
        };
        let palette = self.palette;
        let paragraph = self.text_line(
            &label,
            TextStyle::role(TextRole::Callout, palette.on_accent),
            DRAG_LABEL_WIDTH,
        );
        let pill = Rect::new(
            drag.at.x + 12.0,
            drag.at.y + 8.0,
            paragraph.width() + 20.0,
            paragraph.height() + 8.0,
        );
        items.push(Item::Fill {
            shape: Shape::rounded(pill, pill.height / 2.0),
            paint: palette.accent.into(),
        });
        items.push(Item::Text {
            origin: Point::new(pill.x + 10.0, pill.y + 4.0),
            paragraph,
        });
    }

    fn text_line(&mut self, text: &str, style: TextStyle, width: f32) -> Paragraph {
        let paragraph = self
            .ui
            .context()
            .text
            .paragraph(text, style, None, TextAlign::Start);
        match paragraph.width() <= width {
            true => paragraph,
            false => {
                let mut shortened = text.chars().collect::<Vec<_>>();
                while !shortened.is_empty() {
                    shortened.pop();
                    let candidate = format!("{}…", shortened.iter().collect::<String>().trim_end());
                    let paragraph =
                        self.ui
                            .context()
                            .text
                            .paragraph(&candidate, style, None, TextAlign::Start);
                    if paragraph.width() <= width {
                        return paragraph;
                    }
                }
                paragraph
            }
        }
    }

    fn put_symbol(
        &mut self,
        items: &mut Vec<Item>,
        symbol: Symbol,
        rect: Rect,
        color: bande_toolkit::Color,
    ) {
        if let Some(image) = self.symbols.get(symbol, ICON_PIXELS, color) {
            items.push(Item::Image { rect, image });
        }
    }

    fn paint_sidebar(&mut self, items: &mut Vec<Item>) {
        let palette = self.palette;
        let window_active = self.ui.context().window_active;
        let frame = self.sidebar_frame();
        let row_height = self.sidebar_row_height();
        let heading_height = (SIDEBAR_HEADING * self.scale()).round();
        let content = self
            .sidebar
            .iter()
            .map(|row| match row {
                Row::Heading(..) => heading_height,
                Row::Place { .. } => row_height,
            })
            .sum::<f32>()
            + SIDEBAR_INSET;
        let offset = {
            let scroll = &mut self.scrolls[0];
            scroll.set_extent(content, frame.height);
            scroll.offset(self.ui.context())
        };
        let mut drawn = Vec::new();
        let mut y = frame.y + SIDEBAR_INSET / 2.0 - offset;
        for (index, row) in self.sidebar.clone().iter().enumerate() {
            match row {
                Row::Heading(title, account) => {
                    let style = TextStyle::role(TextRole::Label, palette.ink3);
                    let paragraph =
                        self.text_line(title, style, frame.width - 2.0 * SIDEBAR_TEXT_X);
                    let text_y = y + heading_height - paragraph.height() - 4.0;
                    drawn.push(Item::Text {
                        origin: Point::new(SIDEBAR_TEXT_X, text_y),
                        paragraph,
                    });
                    let offline = account.filter(|account| self.offline(*account));
                    if let Some(account) = offline {
                        let ink = match self.hover == Some(Hit::Offline(account)) {
                            true => palette.ink,
                            false => palette.ink3,
                        };
                        let paragraph = self.text_line(
                            &tr!("Offline"),
                            TextStyle::role(TextRole::Callout, ink),
                            80.0,
                        );
                        let origin = Point::new(
                            frame.width - SIDEBAR_INSET - 8.0 - paragraph.width(),
                            y + heading_height - paragraph.height() - 4.0,
                        );
                        self.hits.push((
                            Rect::new(origin.x, origin.y, paragraph.width(), paragraph.height()),
                            Hit::Offline(account),
                        ));
                        drawn.push(Item::Text { origin, paragraph });
                    }
                    y += heading_height;
                }
                Row::Place {
                    place,
                    title,
                    symbol,
                    unread,
                    depth,
                } => {
                    let rect = Rect::new(
                        SIDEBAR_INSET,
                        y,
                        frame.width - 2.0 * SIDEBAR_INSET,
                        row_height,
                    );
                    let selected = *place == self.place;
                    let dropping = self.drag.is_some_and(|drag| drag.over == Some(index));
                    if dropping {
                        drawn.push(Item::Fill {
                            shape: Shape::rounded(rect, Radius::S),
                            paint: palette.accent.with_opacity(0.25).into(),
                        });
                        drawn.push(Item::Stroke {
                            shape: Shape::rounded(rect.inset(1.0), Radius::S - 1.0),
                            width: 2.0,
                            paint: palette.accent.into(),
                        });
                    } else if selected {
                        drawn.push(Item::Fill {
                            shape: Shape::rounded(rect, Radius::S),
                            paint: palette
                                .sidebar_selection(window_active, self.focus == Pane::Sidebar)
                                .into(),
                        });
                    } else if self.hover == Some(Hit::Place(index)) {
                        drawn.push(Item::Fill {
                            shape: Shape::rounded(rect, Radius::S),
                            paint: palette.well.with_opacity(0.5).into(),
                        });
                    }
                    let x = SIDEBAR_TEXT_X + *depth as f32 * SIDEBAR_INDENT;
                    let tint = match selected {
                        true => palette.accent,
                        false => palette.ink2,
                    };
                    let icon = Rect::new(
                        x,
                        y + ((row_height - ICON_SIZE) / 2.0).round(),
                        ICON_SIZE,
                        ICON_SIZE,
                    );
                    self.put_symbol(&mut drawn, *symbol, icon, tint);
                    let mut right = rect.x + rect.width - 8.0;
                    if *unread > 0 {
                        let badge = self.ui.context().text.paragraph(
                            &unread.to_string(),
                            TextStyle::role(TextRole::Callout, palette.ink3).tabular(),
                            None,
                            TextAlign::End,
                        );
                        right -= badge.width();
                        drawn.push(Item::Text {
                            origin: Point::new(
                                right,
                                y + ((row_height - badge.height()) / 2.0).round(),
                            ),
                            paragraph: badge,
                        });
                        right -= BADGE_GAP;
                    }
                    let text_x = x + ICON_SIZE + ICON_GAP;
                    let paragraph = self.text_line(
                        title,
                        TextStyle::role(TextRole::Body, palette.ink),
                        right - text_x,
                    );
                    drawn.push(Item::Text {
                        origin: Point::new(
                            text_x,
                            y + ((row_height - paragraph.height()) / 2.0).round(),
                        ),
                        paragraph,
                    });
                    if rect.y + rect.height > frame.y && rect.y < frame.y + frame.height {
                        self.hits.push((rect, Hit::Place(index)));
                    }
                    y += row_height;
                }
            }
        }
        items.push(Item::Clip {
            shape: Shape::rect(frame),
            items: drawn,
        });
        let scroll = &self.scrolls[0];
        let cx = self.ui.context();
        scroll.paint_scrollbar(cx, frame, items);
    }

    fn paint_list_header(&mut self, items: &mut Vec<Item>) {
        let palette = self.palette;
        let frame = self.list_header_frame();
        let unread = self.unread_here();
        let count = match unread {
            0 => String::new(),
            unread => trn!("{count} unread", "{count} unread", count = unread),
        };
        let count = self.text_line(
            &count,
            TextStyle::role(TextRole::Callout, palette.ink3).tabular(),
            frame.width / 2.0,
        );
        let title_width = frame.width - 2.0 * LIST_HEADER_INSET - count.width() - BADGE_GAP;
        let title = self.text_line(
            &self.title(),
            TextStyle::role(TextRole::Heading, palette.ink),
            title_width.max(0.0),
        );
        let row_y = frame.y + (LIST_TITLE_HEIGHT - title.height()).max(0.0).round() + 4.0;
        items.push(Item::Text {
            origin: Point::new(frame.x + LIST_HEADER_INSET, row_y),
            paragraph: title,
        });
        items.push(Item::Text {
            origin: Point::new(
                frame.x + frame.width - LIST_HEADER_INSET - count.width(),
                row_y,
            ),
            paragraph: count,
        });
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(
                frame.x,
                frame.y + frame.height - 1.0,
                frame.width,
                1.0,
            )),
            paint: palette.hairline.into(),
        });
    }

    pub(crate) fn empty_list_text(&self) -> (String, String) {
        match (self.search.trim(), self.filter) {
            ("", Filter::All) => (
                tr!("{mailbox} is empty", mailbox = self.title()),
                self.sync_line(),
            ),
            ("", Filter::Unread) => (tr!("No unread messages"), String::new()),
            ("", Filter::Flagged) => (tr!("No flagged messages"), String::new()),
            (search, _) => (
                tr!("No messages match “{search}”", search = search.to_owned()),
                tr!("Search looks in the sender, recipients, subject and text."),
            ),
        }
    }

    fn paint_empty_list(&mut self, items: &mut Vec<Item>, frame: Rect) {
        let palette = self.palette;
        let (heading, detail) = self.empty_list_text();
        let symbol = match &self.place {
            Place::All(role) => Symbol::for_role(Some(*role)),
            Place::Flagged => Symbol::Flag,
            Place::Mailbox(_) => Symbol::Folder,
        };
        let width = (frame.width - 2.0 * LIST_TEXT_X).max(0.0);
        let heading = self.ui.context().text.paragraph(
            &heading,
            TextStyle::role(TextRole::BodyStrong, palette.ink2),
            Some(width),
            TextAlign::Center,
        );
        let detail = self.ui.context().text.paragraph(
            &detail,
            TextStyle::role(TextRole::Callout, palette.ink3),
            Some(width),
            TextAlign::Center,
        );
        let mut y = frame.y + (frame.height / 3.0).round();
        self.put_symbol(
            items,
            symbol,
            Rect::new(
                frame.x + ((frame.width - EMPTY_SYMBOL) / 2.0).round(),
                y,
                EMPTY_SYMBOL,
                EMPTY_SYMBOL,
            ),
            palette.ink3,
        );
        y += EMPTY_SYMBOL + 12.0;
        let heading_height = heading.height();
        items.push(Item::Text {
            origin: Point::new(frame.x + LIST_TEXT_X, y),
            paragraph: heading,
        });
        items.push(Item::Text {
            origin: Point::new(frame.x + LIST_TEXT_X, y + heading_height + 4.0),
            paragraph: detail,
        });
    }

    fn paint_list(&mut self, items: &mut Vec<Item>) {
        let palette = self.palette;
        let frame = self.list_frame();
        let row_height = self.row_height();
        let offset = {
            let content = self.row_top(self.messages.len(), row_height);
            let scroll = &mut self.scrolls[1];
            scroll.set_extent(content, frame.height);
            scroll.offset(self.ui.context())
        };
        let mut drawn = Vec::new();
        if self.messages.is_empty() && frame.width > 0.0 {
            self.paint_empty_list(&mut drawn, frame);
        }
        let first = self.first_row_after(offset, row_height);
        let visible = (frame.height / row_height).ceil() as usize + 1;
        let now = self.now.unwrap_or_else(Timestamp::now);
        let shown = self
            .messages
            .iter()
            .enumerate()
            .skip(first)
            .take(visible)
            .map(|(index, summary)| (index, summary.clone()))
            .collect::<Vec<_>>();
        let groups = self
            .groups
            .iter()
            .filter(|(start, _)| (first..first + visible).contains(start))
            .cloned()
            .collect::<Vec<_>>();
        for (start, label) in groups {
            let y = frame.y + self.row_top(start, row_height) - offset - GROUP_HEIGHT;
            let paragraph = self.text_line(
                &label,
                TextStyle::role(TextRole::Label, palette.ink3),
                frame.width - 2.0 * LIST_TEXT_X,
            );
            drawn.push(Item::Text {
                origin: Point::new(
                    frame.x + LIST_TEXT_X,
                    y + (GROUP_HEIGHT - paragraph.height() - 4.0).round(),
                ),
                paragraph,
            });
        }
        for (index, summary) in shown {
            let y = frame.y + self.row_top(index, row_height) - offset;
            let rect = Rect::new(frame.x, y, frame.width, row_height);
            self.hits.push((rect, Hit::Message(summary.id)));
            match self.swipe.filter(|(id, _)| *id == summary.id) {
                Some((_, moved)) => {
                    let (paint, symbol, x) = match moved > 0.0 {
                        true => (palette.success, Symbol::Archive, rect.x + LIST_TEXT_X),
                        false => (
                            palette.danger,
                            Symbol::Trash,
                            rect.x + rect.width - LIST_TEXT_X - ICON_SIZE,
                        ),
                    };
                    drawn.push(Item::Fill {
                        shape: Shape::rect(rect),
                        paint: paint.into(),
                    });
                    let icon = Rect::new(
                        x,
                        rect.y + ((rect.height - ICON_SIZE) / 2.0).round(),
                        ICON_SIZE,
                        ICON_SIZE,
                    );
                    self.put_symbol(&mut drawn, symbol, icon, palette.on_accent);
                    let mut row = vec![Item::Fill {
                        shape: Shape::rect(rect),
                        paint: palette.sheet.into(),
                    }];
                    self.paint_row(&mut row, &summary, rect, now);
                    drawn.push(Item::Translate {
                        offset: Point::new(moved, 0.0),
                        items: row,
                    });
                }
                None => self.paint_row(&mut drawn, &summary, rect, now),
            }
        }
        items.push(Item::Clip {
            shape: Shape::rect(frame),
            items: drawn,
        });
        let sheet = self.sheet_frame();
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(frame.x + frame.width, sheet.y, 1.0, sheet.height)),
            paint: palette.hairline.into(),
        });
        let scroll = &self.scrolls[1];
        let cx = self.ui.context();
        scroll.paint_scrollbar(cx, frame, items);
    }

    fn paint_row(&mut self, items: &mut Vec<Item>, summary: &Summary, rect: Rect, now: Timestamp) {
        let palette = self.palette;
        let selected = self.is_chosen(summary.id);
        let active = selected
            && self.focus == Pane::List
            && !self.ui.has_sheet()
            && self.ui.context().window_active;
        let highlight = Rect::new(
            rect.x + ROW_INSET,
            rect.y + 1.0,
            rect.width - 2.0 * ROW_INSET,
            rect.height - 2.0,
        );
        let (ink, ink2, ink3) = match (selected, active) {
            (true, true) => (
                palette.on_accent,
                palette.on_accent.with_opacity(0.85),
                palette.on_accent.with_opacity(0.75),
            ),
            _ => (palette.ink, palette.ink2, palette.ink3),
        };
        if selected {
            items.push(Item::Fill {
                shape: Shape::rounded(highlight, Radius::M),
                paint: match active {
                    true => palette.accent.into(),
                    false => palette.well.into(),
                },
            });
        } else {
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    rect.x + LIST_TEXT_X,
                    rect.y + rect.height - 1.0,
                    rect.width - LIST_TEXT_X - LIST_TRAILING,
                    1.0,
                )),
                paint: palette.hairline.into(),
            });
        }
        let scale = self.scale();
        let indent = match self.conversations.children.contains(&summary.id) {
            true => CHILD_INDENT,
            false => 0.0,
        };
        let text_x = rect.x + LIST_TEXT_X + indent;
        let right = rect.x + rect.width - LIST_TRAILING;
        let mut y = rect.y + LIST_PADDING;
        let date = self.ui.context().text.paragraph(
            &list_date(summary.date, now, &self.zone, &self.formats),
            TextStyle::role(TextRole::Callout, ink3),
            None,
            TextAlign::End,
        );
        let line = (18.0 * scale).round();
        let hovered = self.swipe.is_none()
            && matches!(&self.hover, Some(Hit::Message(id) | Hit::Quick(id, _)) if *id == summary.id);
        let mut date_x = match hovered {
            true => self.paint_quick(items, summary.id, right, y, line, ink2),
            false => {
                let x = right - date.width();
                items.push(Item::Text {
                    origin: Point::new(x, y + ((line - date.height()) / 2.0).round()),
                    paragraph: date,
                });
                x
            }
        };
        if let Some(count) = self.conversations.count(summary.id) {
            let paragraph = self.ui.context().text.paragraph(
                &count.to_string(),
                TextStyle::role(TextRole::Callout, ink2).tabular(),
                None,
                TextAlign::Center,
            );
            let width = (paragraph.width() + 12.0).max(line + 4.0);
            let capsule = Rect::new(date_x - width - 6.0, y, width, line);
            items.push(Item::Stroke {
                shape: Shape::rounded(capsule.inset(0.5), line / 2.0),
                width: 1.0,
                paint: ink3.into(),
            });
            items.push(Item::Text {
                origin: Point::new(
                    capsule.x + ((width - paragraph.width()) / 2.0).round(),
                    y + ((line - paragraph.height()) / 2.0).round(),
                ),
                paragraph,
            });
            self.hits.push((capsule, Hit::Conversation(summary.id)));
            date_x = capsule.x;
        }
        let unread = !summary.flags.seen
            || self
                .conversations
                .collapsed(summary.id)
                .is_some_and(|earlier| earlier.iter().any(|summary| !summary.flags.seen));
        if unread {
            let dot = Rect::new(
                rect.x + indent + (LIST_TEXT_X - DOT) / 2.0,
                y + ((line - DOT) / 2.0).round(),
                DOT,
                DOT,
            );
            items.push(Item::Fill {
                shape: Shape::rounded(dot, DOT / 2.0),
                paint: match active {
                    true => palette.on_accent.into(),
                    false => palette.accent.into(),
                },
            });
        }
        let sender = match summary.sender.shown() {
            "" => tr!("No Sender"),
            shown => shown.to_owned(),
        };
        let compact = self.settings.density == Density::Compact;
        let sender = self.text_line(
            &sender,
            TextStyle::role(TextRole::BodyStrong, ink),
            match compact {
                true => ((date_x - text_x) * 0.45).round(),
                false => date_x - text_x - 8.0,
            },
        );
        let sender_end = text_x + sender.width();
        items.push(Item::Text {
            origin: Point::new(text_x, y),
            paragraph: sender,
        });
        if !compact {
            y += line;
        }
        let mut subject_right = match compact {
            true => date_x - 8.0,
            false => right,
        };
        let symbols = [
            (
                summary.flags.flagged,
                Symbol::Flag,
                match active {
                    true => ink,
                    false => FLAG_COLORS
                        .get(usize::from(summary.flags.color))
                        .copied()
                        .map_or(palette.warning, Color::hex),
                },
            ),
            (summary.attachments, Symbol::Attachment, ink3),
        ];
        for (shown, symbol, color) in symbols {
            if shown {
                subject_right -= SMALL_SYMBOL;
                let icon = Rect::new(
                    subject_right,
                    y + ((line - SMALL_SYMBOL) / 2.0).round(),
                    SMALL_SYMBOL,
                    SMALL_SYMBOL,
                );
                self.put_symbol(items, symbol, icon, color);
                subject_right -= 6.0;
            }
        }
        let subject = match summary.subject.as_str() {
            "" => tr!("No Subject"),
            subject => subject.to_owned(),
        };
        let subject_x = match compact {
            true => sender_end + 8.0,
            false => text_x,
        };
        let subject = self.text_line(
            &subject,
            TextStyle::role(TextRole::Body, if compact { ink2 } else { ink }),
            (subject_right - subject_x).max(0.0),
        );
        items.push(Item::Text {
            origin: Point::new(subject_x, y),
            paragraph: subject,
        });
        y += line;
        let preview_lines = match compact {
            true => 1.0,
            false => 2.0,
        };
        let preview_height = (16.0 * preview_lines * scale).round();
        let preview = self.ui.context().text.paragraph(
            &summary.preview,
            TextStyle::role(TextRole::Callout, ink2),
            Some(right - text_x),
            TextAlign::Start,
        );
        items.push(Item::Clip {
            shape: Shape::rect(Rect::new(text_x, y, right - text_x, preview_height)),
            items: vec![Item::Text {
                origin: Point::new(text_x, y),
                paragraph: preview,
            }],
        });
        if let Some(color) = self.account_mark(summary.account) {
            items.push(Item::Fill {
                shape: Shape::rounded(
                    Rect::new(
                        rect.x + ACCOUNT_MARK_INSET,
                        rect.y + LIST_PADDING,
                        ACCOUNT_MARK,
                        rect.height - 2.0 * LIST_PADDING,
                    ),
                    ACCOUNT_MARK / 2.0,
                ),
                paint: color.into(),
            });
        }
    }

    fn paint_quick(
        &mut self,
        items: &mut Vec<Item>,
        id: MessageId,
        right: f32,
        y: f32,
        line: f32,
        ink: Color,
    ) -> f32 {
        let mut x = right;
        for (quick, symbol) in [
            (Quick::Flag, Symbol::Flag),
            (Quick::Delete, Symbol::Trash),
            (Quick::Archive, Symbol::Archive),
        ] {
            x -= QUICK_SIZE;
            let rect = Rect::new(
                x,
                y + ((line - QUICK_SIZE) / 2.0).round(),
                QUICK_SIZE,
                QUICK_SIZE,
            );
            if self.hover == Some(Hit::Quick(id, quick)) {
                items.push(Item::Fill {
                    shape: Shape::rounded(rect, Radius::S),
                    paint: self.palette.well.into(),
                });
            }
            let icon = Rect::new(
                rect.x + ((QUICK_SIZE - SMALL_SYMBOL) / 2.0).round(),
                rect.y + ((QUICK_SIZE - SMALL_SYMBOL) / 2.0).round(),
                SMALL_SYMBOL,
                SMALL_SYMBOL,
            );
            self.put_symbol(items, symbol, icon, ink);
            self.hits.push((rect, Hit::Quick(id, quick)));
            x -= QUICK_GAP;
        }
        x
    }

    fn account_mark(&self, account: AccountId) -> Option<Color> {
        if self.accounts.len() < 2 || matches!(self.place, Place::Mailbox(_)) {
            return None;
        }
        let index = self
            .accounts
            .iter()
            .position(|(known, _)| known.id == account)?;
        FLAG_COLORS
            .get(ACCOUNT_COLORS[index % ACCOUNT_COLORS.len()])
            .map(|hex| Color::hex(*hex))
    }

    fn paint_reader(&mut self, items: &mut Vec<Item>) {
        let palette = self.palette;
        let frame = self.reader_frame();
        if frame.width <= 0.0 {
            return;
        }
        if self.reader.thread.is_none() {
            let chosen = self.also.len() + usize::from(self.selected.is_some());
            let (message, hint) = match (self.messages.len(), chosen) {
                (0, _) => (String::new(), String::new()),
                (_, 0 | 1) => (
                    tr!("No message selected"),
                    tr!("Up and Down choose a message. Ctrl+N starts a new one."),
                ),
                (_, count) => (
                    tr!("{count} messages selected", count = count),
                    String::new(),
                ),
            };
            let paragraph = self.ui.context().text.paragraph(
                &message,
                TextStyle::role(TextRole::Heading, palette.ink3),
                Some(frame.width),
                TextAlign::Center,
            );
            let top = frame.y + (frame.height / 2.0 - paragraph.height()).round();
            let hint = self.ui.context().text.paragraph(
                &hint,
                TextStyle::role(TextRole::Callout, palette.ink3),
                Some(frame.width),
                TextAlign::Center,
            );
            items.push(Item::Text {
                origin: Point::new(frame.x, top + paragraph.height() + 8.0),
                paragraph: hint,
            });
            items.push(Item::Text {
                origin: Point::new(frame.x, top),
                paragraph,
            });
            return;
        }
        let zone = self.zone.clone();
        let formats = self.formats.clone();
        let look = Look {
            palette: &palette,
            symbols: &mut self.symbols,
            clock: Clock {
                zone: &zone,
                formats: &formats,
                now: self.now.unwrap_or_else(Timestamp::now),
            },
            focused: self.focus == Pane::Reader,
            sheet: self.ui.has_sheet(),
        };
        items.extend(self.reader.paint(self.ui.context(), look, frame));
        let targets = self.reader.visible_targets(self.ui.context(), frame);
        self.hits.extend(
            targets
                .into_iter()
                .map(|(rect, target)| (rect, Hit::Reader(target))),
        );
        if let Some(Hit::Reader(Target::Link(url, ..))) = &self.hover {
            let url = url.clone();
            paint_link_target(self.ui.context(), &palette, items, frame, &url);
        }
    }

    pub(crate) fn scroll(&mut self, scroll: &Scroll) -> bool {
        let at = scroll.position;
        if scroll.source == ScrollSource::Finger
            && self.list_frame().contains(at)
            && self.swiped(scroll)
        {
            return true;
        }
        let index = if self.sidebar_frame().contains(at) {
            0
        } else if self.list_frame().contains(at) {
            1
        } else if self.reader_frame().contains(at) {
            let cx = self.ui.context();
            return self.reader.scroll.scroll(cx, scroll);
        } else {
            return false;
        };
        let state = &mut self.scrolls[index];
        let cx = self.ui.context();
        state.scroll(cx, scroll)
    }

    fn swiped(&mut self, scroll: &Scroll) -> bool {
        let sideways = scroll.delta.x.abs() > 1.5 * scroll.delta.y.abs();
        let row = match self.hit(scroll.position) {
            Some(Hit::Message(id) | Hit::Quick(id, _)) => Some(id),
            _ => None,
        };
        match (self.swipe, scroll.phase) {
            (Some((id, moved)), ScrollPhase::Ended) => {
                self.swipe = None;
                if moved.abs() >= SWIPE_COMMIT {
                    let quick = match moved > 0.0 {
                        true => Quick::Archive,
                        false => Quick::Delete,
                    };
                    self.quick(id, quick);
                }
                true
            }
            (Some((id, moved)), _) => {
                let width = self.list_frame().width;
                self.swipe = Some((id, (moved - scroll.delta.x).clamp(-width, width)));
                true
            }
            (None, ScrollPhase::Ended) => false,
            (None, _) if sideways => match row {
                Some(id) => {
                    self.swipe = Some((id, -scroll.delta.x));
                    true
                }
                None => false,
            },
            (None, _) => false,
        }
    }

    pub(crate) fn page_reader(&mut self) -> bool {
        self.scroll_reader(Key::PageDown)
    }

    pub(crate) fn scroll_reader(&mut self, key: Key) -> bool {
        let frame = self.reader_frame();
        self.reader.scroll_key(self.ui.context(), frame, key)
    }

    pub(crate) fn rows_per_page(&mut self) -> isize {
        let rows = self.list_frame().height / self.row_height();
        (rows.floor() as isize).max(1)
    }

    pub(crate) fn sidebar_rows_per_page(&mut self) -> isize {
        let rows = self.sidebar_frame().height / self.sidebar_row_height();
        (rows.floor() as isize).max(1)
    }

    pub(crate) fn reveal_place(&mut self, index: usize) {
        let row_height = self.sidebar_row_height();
        let heading_height = (SIDEBAR_HEADING * self.scale()).round();
        let height = |row: &Row| match row {
            Row::Heading(..) => heading_height,
            Row::Place { .. } => row_height,
        };
        let top = SIDEBAR_INSET / 2.0 + self.sidebar.iter().take(index).map(height).sum::<f32>();
        let state = &mut self.scrolls[0];
        let cx = self.ui.context();
        state.reveal(cx, top, top + row_height);
    }

    pub(crate) fn reveal_selected(&mut self) {
        let Some(index) = self
            .selected
            .and_then(|id| self.messages.iter().position(|summary| summary.id == id))
        else {
            return;
        };
        let row_height = self.row_height();
        let top = self.row_top(index, row_height);
        let state = &mut self.scrolls[1];
        let cx = self.ui.context();
        state.reveal(cx, top, top + row_height);
    }
}
