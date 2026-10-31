use std::path::PathBuf;

use bande_toolkit::ui::ScrollState;
use bande_toolkit::{Color, Item, Point, Radius, Rect, Shape, TextAlign, TextStyle};

use crate::L10N;
use crate::browser::{Browser, Location};
use crate::view::{FilesView, Hit};
use crate::views::ViewMode;

pub const TAB_BAR_HEIGHT: f32 = 36.0;
const TAB_HEIGHT: f32 = 28.0;
const TAB_INSET: f32 = 4.0;
const TAB_GAP: f32 = 4.0;
const NEW_TAB_WIDTH: f32 = 28.0;
const CLOSE_SIZE: f32 = 16.0;
const CLOSE_INSET: f32 = 6.0;
const LABEL_SIZE: f32 = 12.0;
const LABEL_INSET: f32 = 26.0;
const SYMBOL_PIXELS: u32 = 32;

pub(crate) struct Tab {
    browser: Browser,
    mode: ViewMode,
    icon_size: u32,
    shown_root: Option<Location>,
    grid_scroll: (Option<Location>, ScrollState),
    scrolls: Vec<(Location, ScrollState)>,
}

pub(crate) fn location_title(location: &Location) -> String {
    match location {
        Location::Trash => L10N.tr("trash"),
        Location::Recents => L10N.tr("recents"),
        Location::Search(folder) => L10N.tr_args(
            "search-title",
            [(
                "folder",
                location_title(&Location::Folder(folder.clone())).into(),
            )],
        ),
        Location::Folder(folder) => folder.file_name().map_or_else(
            || folder.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        ),
    }
}

impl FilesView {
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    pub fn active_tab(&self) -> usize {
        self.active_tab
    }

    pub(crate) fn shows_tab_bar(&self) -> bool {
        self.tabs.len() > 1
    }

    pub(crate) fn tab_bar_height(&self) -> f32 {
        if self.shows_tab_bar() {
            TAB_BAR_HEIGHT
        } else {
            0.0
        }
    }

    fn park(&mut self, browser: Browser) -> Tab {
        Tab {
            browser: std::mem::replace(&mut self.browser, browser),
            mode: self.mode,
            icon_size: self.icon_size,
            shown_root: self.shown_root.take(),
            grid_scroll: std::mem::take(&mut self.grid_scroll),
            scrolls: std::mem::take(&mut self.scrolls),
        }
    }

    fn leave_tab(&mut self) {
        self.commit_rename();
        self.end_search();
        self.quick_look = None;
        self.band = None;
        self.table_synced = None;
    }

    fn show_tab(&mut self) {
        self.refresh_size_slider();
        self.refresh_view_control();
        self.selection_changed();
        self.focus_content();
    }

    pub(crate) fn open_tab(&mut self, location: Location) {
        self.leave_tab();
        let mut browser = Browser::open(location, self.browser.trash().clone());
        if self.mode != ViewMode::Columns {
            browser.set_single(true);
        }
        let parked = self.park(browser);
        self.tabs[self.active_tab] = Some(parked);
        self.active_tab += 1;
        self.tabs.insert(self.active_tab, None);
        self.show_tab();
    }

    pub(crate) fn select_tab(&mut self, index: usize) {
        if index == self.active_tab || index >= self.tabs.len() {
            return;
        }
        let Some(tab) = self.tabs[index].take() else {
            return;
        };
        self.leave_tab();
        let parked = self.park(tab.browser);
        self.tabs[self.active_tab] = Some(parked);
        self.active_tab = index;
        self.mode = tab.mode;
        self.icon_size = tab.icon_size;
        self.shown_root = tab.shown_root;
        self.grid_scroll = tab.grid_scroll;
        self.scrolls = tab.scrolls;
        self.browser.reload();
        self.show_tab();
    }

    pub(crate) fn step_tab(&mut self, forward: bool) {
        let count = self.tabs.len();
        let next = match forward {
            true => (self.active_tab + 1) % count,
            false => (self.active_tab + count - 1) % count,
        };
        self.select_tab(next);
    }

    pub fn close_tab(&mut self, index: usize) -> bool {
        if self.tabs.len() < 2 || index >= self.tabs.len() {
            return false;
        }
        if index == self.active_tab {
            let neighbour = if index + 1 < self.tabs.len() {
                index + 1
            } else {
                index - 1
            };
            self.select_tab(neighbour);
        }
        self.tabs.remove(index);
        if self.active_tab > index {
            self.active_tab -= 1;
        }
        self.show_tab();
        true
    }

    pub(crate) fn tab_location(&self, index: usize) -> Option<Location> {
        match self.tabs.get(index)? {
            Some(tab) => Some(tab.browser.root().clone()),
            None => Some(self.browser.root().clone()),
        }
    }

    pub fn tab_titles(&self) -> Vec<String> {
        (0..self.tabs.len())
            .filter_map(|index| self.tab_location(index))
            .map(|location| location_title(&location))
            .collect()
    }

    pub(crate) fn open_selection_in_tabs(&mut self) {
        let folders = self
            .browser
            .selection()
            .iter()
            .filter(|entry| entry.is_folder() && entry.trashed.is_none())
            .map(|entry| entry.path.clone())
            .collect::<Vec<PathBuf>>();
        for folder in folders {
            self.open_tab(Location::Folder(folder));
        }
    }

    fn tab_rects(&self) -> (Vec<Rect>, Rect) {
        let sheet = self.sheet_frame();
        let content = self.content_frame();
        let top = sheet.y + TAB_INSET;
        let left = content.x + TAB_INSET;
        let available = (content.width - 2.0 * TAB_INSET - NEW_TAB_WIDTH - TAB_GAP).max(0.0);
        let count = self.tabs.len().max(1) as f32;
        let width = (available + TAB_GAP) / count;
        let tabs = (0..self.tabs.len())
            .map(|index| {
                let start = (index as f32 * width).round();
                let end = ((index + 1) as f32 * width - TAB_GAP).round();
                Rect::new(left + start, top, end - start, TAB_HEIGHT)
            })
            .collect();
        (
            tabs,
            Rect::new(left + available + TAB_GAP, top, NEW_TAB_WIDTH, TAB_HEIGHT),
        )
    }

    fn centred(&mut self, text: &str, size: f32, color: Color, strong: bool, area: Rect) -> Item {
        let weight = match strong {
            true => TextStyle::SEMIBOLD,
            false => TextStyle::REGULAR,
        };
        let paragraph = self.text().paragraph(
            text,
            TextStyle::new(size, color).weight(weight),
            Some(area.width),
            TextAlign::Center,
        );
        Item::Clip {
            shape: Shape::rect(area),
            items: vec![Item::Text {
                origin: Point::new(
                    area.x,
                    area.y + ((area.height - paragraph.height()) / 2.0).round(),
                ),
                paragraph,
            }],
        }
    }

    pub(crate) fn paint_tab_bar(&mut self, items: &mut Vec<Item>) {
        if !self.shows_tab_bar() {
            return;
        }
        let palette = self.palette;
        let (tabs, new_tab) = self.tab_rects();
        let titles = self.tab_titles();
        for (index, (tab, title)) in tabs.iter().zip(titles).enumerate() {
            let active = index == self.active_tab;
            let hovered = matches!(
                self.hover,
                Some(Hit::Tab(hovered) | Hit::CloseTab(hovered)) if hovered == index
            );
            let fill = match (active, hovered) {
                (true, _) => Some(palette.well),
                (false, true) => Some(palette.well.with_opacity(0.5)),
                (false, false) => None,
            };
            if let Some(fill) = fill {
                items.push(Item::Fill {
                    shape: Shape::rounded(*tab, Radius::S),
                    paint: fill.into(),
                });
            }
            let color = match active {
                true => palette.ink,
                false => palette.ink2,
            };
            let label = Rect::new(
                tab.x + LABEL_INSET,
                tab.y,
                (tab.width - 2.0 * LABEL_INSET).max(0.0),
                TAB_HEIGHT,
            );
            items.push(self.centred(&title, LABEL_SIZE, color, active, label));
            self.hits.push((*tab, Hit::Tab(index)));
            let close = Rect::new(
                tab.x + CLOSE_INSET,
                tab.y + (TAB_HEIGHT - CLOSE_SIZE) / 2.0,
                CLOSE_SIZE,
                CLOSE_SIZE,
            );
            if hovered {
                if self.hover == Some(Hit::CloseTab(index)) {
                    items.push(Item::Fill {
                        shape: Shape::rounded(close, Radius::XS),
                        paint: palette.well.mix(palette.ink, 0.08).into(),
                    });
                }
                self.symbol("window-close-symbolic", close, palette.ink2, items);
            }
            self.hits.push((close, Hit::CloseTab(index)));
        }
        if self.hover == Some(Hit::NewTab) {
            items.push(Item::Fill {
                shape: Shape::rounded(new_tab, Radius::S),
                paint: palette.well.with_opacity(0.5).into(),
            });
        }
        let plus = Rect::new(
            new_tab.x + (NEW_TAB_WIDTH - CLOSE_SIZE) / 2.0,
            new_tab.y + (TAB_HEIGHT - CLOSE_SIZE) / 2.0,
            CLOSE_SIZE,
            CLOSE_SIZE,
        );
        self.symbol("list-add-symbolic", plus, palette.ink2, items);
        self.hits.push((new_tab, Hit::NewTab));
    }

    fn symbol(&mut self, name: &str, rect: Rect, color: Color, items: &mut Vec<Item>) {
        if let Some(image) = self.icons.symbol(&[name], SYMBOL_PIXELS, color) {
            items.push(Item::Image { rect, image });
        }
    }
}
