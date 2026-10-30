use std::path::PathBuf;

use unbound_toolkit::ui::ScrollState;
use unbound_toolkit::{Color, Item, Point, Rect, Shape, TextAlign, TextStyle};

use crate::L10N;
use crate::browser::{Browser, Location};
use crate::toolbar::TOOLBAR_HEIGHT;
use crate::view::{FilesView, Hit, SIDEBAR_WIDTH};
use crate::views::ViewMode;

pub const TAB_BAR_HEIGHT: f32 = 28.0;
const NEW_TAB_WIDTH: f32 = 32.0;
const CLOSE_SIZE: f32 = 16.0;
const CLOSE_INSET: f32 = 6.0;
const LABEL_SIZE: f32 = 12.0;
const LABEL_INSET: f32 = 26.0;

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
        self.table_revision = None;
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
        let content = self.content_frame();
        let available = (content.width - NEW_TAB_WIDTH).max(0.0);
        let width = available / self.tabs.len().max(1) as f32;
        let tabs = (0..self.tabs.len())
            .map(|index| {
                let left = (index as f32 * width).round();
                let right = ((index + 1) as f32 * width).round();
                Rect::new(
                    SIDEBAR_WIDTH + left,
                    TOOLBAR_HEIGHT,
                    right - left,
                    TAB_BAR_HEIGHT,
                )
            })
            .collect();
        (
            tabs,
            Rect::new(
                SIDEBAR_WIDTH + available,
                TOOLBAR_HEIGHT,
                NEW_TAB_WIDTH,
                TAB_BAR_HEIGHT,
            ),
        )
    }

    fn centred(&mut self, text: &str, size: f32, color: Color, area: Rect) -> Item {
        let paragraph = self.text().paragraph(
            text,
            TextStyle::new(size, color),
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
        let content = self.content_frame();
        let bar = Rect::new(content.x, TOOLBAR_HEIGHT, content.width, TAB_BAR_HEIGHT);
        items.push(Item::Fill {
            shape: Shape::rect(bar),
            paint: palette.surface_elevated.into(),
        });
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(
                bar.x,
                bar.y + TAB_BAR_HEIGHT - 1.0,
                bar.width,
                1.0,
            )),
            paint: palette.separator_color.into(),
        });
        let titles = self.tab_titles();
        for (index, (tab, title)) in tabs.iter().zip(titles).enumerate() {
            let active = index == self.active_tab;
            let hovered = matches!(
                self.hover,
                Some(Hit::Tab(hovered) | Hit::CloseTab(hovered)) if hovered == index
            );
            if active {
                items.push(Item::Fill {
                    shape: Shape::rect(*tab),
                    paint: palette.surface_background.into(),
                });
            } else if hovered {
                items.push(Item::Fill {
                    shape: Shape::rect(Rect::new(tab.x, tab.y, tab.width, TAB_BAR_HEIGHT - 1.0)),
                    paint: palette.label_muted.with_alpha(24).into(),
                });
            }
            if index + 1 < tabs.len() {
                items.push(Item::Fill {
                    shape: Shape::rect(Rect::new(
                        tab.x + tab.width - 1.0,
                        tab.y,
                        1.0,
                        TAB_BAR_HEIGHT,
                    )),
                    paint: palette.separator_color.into(),
                });
            }
            let color = match active {
                true => palette.label_primary,
                false => palette.label_muted,
            };
            let label = Rect::new(
                tab.x + LABEL_INSET,
                tab.y,
                (tab.width - 2.0 * LABEL_INSET).max(0.0),
                TAB_BAR_HEIGHT,
            );
            items.push(self.centred(&title, LABEL_SIZE, color, label));
            self.hits.push((*tab, Hit::Tab(index)));
            let close = Rect::new(
                tab.x + CLOSE_INSET,
                tab.y + (TAB_BAR_HEIGHT - CLOSE_SIZE) / 2.0,
                CLOSE_SIZE,
                CLOSE_SIZE,
            );
            if hovered {
                if self.hover == Some(Hit::CloseTab(index)) {
                    items.push(Item::Fill {
                        shape: Shape::rounded(close, CLOSE_SIZE / 2.0),
                        paint: palette.label_muted.with_alpha(50).into(),
                    });
                }
                items.push(self.centred("\u{00D7}", 14.0, palette.label_muted, close));
            }
            self.hits.push((close, Hit::CloseTab(index)));
        }
        if self.hover == Some(Hit::NewTab) {
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    new_tab.x,
                    new_tab.y,
                    new_tab.width,
                    TAB_BAR_HEIGHT - 1.0,
                )),
                paint: palette.label_muted.with_alpha(24).into(),
            });
        }
        items.push(self.centred("+", 16.0, palette.label_muted, new_tab));
        self.hits.push((new_tab, Hit::NewTab));
    }
}
