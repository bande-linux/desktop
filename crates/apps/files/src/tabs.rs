use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use bande_app::{TabDrop, move_tab};
use bande_toolkit::ui::{
    Pointer, ScrollState, TabAction, TabLabel, TabStripLayout, TabsShown, frame_time,
};
use bande_toolkit::{Item, Point};

use crate::L10N;
use crate::browser::{Browser, Location};
use crate::controls::Press;
use crate::toolbar::TOOLBAR_HEIGHT;
use crate::view::{FilesView, Hit, Request};
use crate::views::ViewMode;

pub const TAB_BAR_HEIGHT: f32 = 36.0;
const TAB_INSET: f32 = 4.0;

static NEXT_TAB_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) struct Tab {
    browser: Browser,
    mode: ViewMode,
    icon_size: u32,
    shown_root: Option<Location>,
    grid_scroll: crate::grid::GridScroll,
    scrolls: Vec<(Location, ScrollState)>,
}

pub(crate) struct TabSlot {
    id: u64,
    parked: Option<Tab>,
}

impl TabSlot {
    pub(crate) fn shown() -> Self {
        Self {
            id: NEXT_TAB_ID.fetch_add(1, Ordering::Relaxed),
            parked: None,
        }
    }
}

pub struct CarriedTab {
    id: u64,
    tab: Box<Tab>,
}

impl CarriedTab {
    pub fn id(&self) -> u64 {
        self.id
    }
}

pub(crate) fn location_title(location: &Location) -> String {
    match location {
        Location::Trash => L10N.tr("trash"),
        Location::Recents => L10N.tr("recents"),
        Location::Server(url) => url.clone(),
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SavedTab {
    pub location: Location,
    mode: ViewMode,
    icon_size: u32,
    selected: Vec<PathBuf>,
}

impl SavedTab {
    fn of(browser: &Browser, mode: ViewMode, icon_size: u32) -> Self {
        Self {
            location: match browser.current_location() {
                Location::Search(folder) => Location::Folder(folder),
                location => location,
            },
            mode,
            icon_size,
            selected: browser.selected_paths(),
        }
    }

    pub fn still_there(&self) -> bool {
        match &self.location {
            Location::Folder(folder) => bande_volumes::quick_is_dir(folder),
            Location::Trash | Location::Recents | Location::Search(_) => true,
            Location::Server(_) => false,
        }
    }
}

impl FilesView {
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
        self.close_quick_look();
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
        browser.set_show_hidden(self.show_hidden);
        browser.set_folders_first(self.folders_first);
        let parked = self.park(browser);
        self.tabs[self.active_tab].parked = Some(parked);
        self.active_tab += 1;
        self.tabs.insert(self.active_tab, TabSlot::shown());
        self.show_tab();
    }

    pub fn saved_tabs(&self) -> Vec<SavedTab> {
        self.tabs
            .iter()
            .map(|slot| match &slot.parked {
                Some(tab) => SavedTab::of(&tab.browser, tab.mode, tab.icon_size),
                None => SavedTab::of(&self.browser, self.mode, self.icon_size),
            })
            .collect()
    }

    pub fn restore_tabs(&mut self, saved: &[SavedTab], active: usize) {
        if let Some(first) = saved.first() {
            self.restore_tab_view(first);
        }
        for tab in saved.iter().skip(1) {
            self.open_tab(tab.location.clone());
            self.restore_tab_view(tab);
        }
        self.select_tab(active);
    }

    fn restore_tab_view(&mut self, tab: &SavedTab) {
        self.set_mode(tab.mode);
        self.set_icon_size(tab.icon_size);
        self.browser.reveal_all(&tab.selected);
    }

    pub(crate) fn select_tab(&mut self, index: usize) {
        if index == self.active_tab || index >= self.tabs.len() {
            return;
        }
        let Some(tab) = self.tabs[index].parked.take() else {
            return;
        };
        self.show_parked(tab, |view, parked| {
            view.tabs[view.active_tab].parked = Some(parked);
            view.active_tab = index;
        });
    }

    fn show_parked(&mut self, tab: Tab, place: impl FnOnce(&mut Self, Tab)) {
        self.leave_tab();
        let parked = self.park(tab.browser);
        place(self, parked);
        self.mode = tab.mode;
        self.icon_size = tab.icon_size;
        self.shown_root = tab.shown_root;
        self.grid_scroll = tab.grid_scroll;
        self.scrolls = tab.scrolls;
        self.browser.set_show_hidden(self.show_hidden);
        self.browser.set_folders_first(self.folders_first);
        self.browser.reload();
        self.show_tab();
    }

    pub(crate) fn step_tab(&mut self, forward: bool) {
        let count = self.tabs.len();
        if count < 2 {
            return;
        }
        let next = match forward {
            true => (self.active_tab + 1) % count,
            false => (self.active_tab + count - 1) % count,
        };
        self.select_tab(next);
    }

    pub fn close_tab(&mut self, index: usize) -> bool {
        self.tabs.len() > 1 && self.remove_tab(index).is_some()
    }

    fn remove_tab(&mut self, index: usize) -> Option<TabSlot> {
        if index >= self.tabs.len() {
            return None;
        }
        if index == self.active_tab {
            let neighbour = if index + 1 < self.tabs.len() {
                index + 1
            } else {
                index.checked_sub(1)?
            };
            self.select_tab(neighbour);
        }
        let removed = self.tabs.remove(index);
        if self.active_tab > index {
            self.active_tab -= 1;
        }
        self.show_tab();
        Some(removed)
    }

    pub fn take_tab(&mut self, index: usize) -> Option<CarriedTab> {
        let (id, tab) = if self.tabs.len() == 1 {
            let id = self.tabs.get(index)?.id;
            self.tabs.clear();
            self.active_tab = 0;
            self.leave_tab();
            let holder = Browser::open(Location::Recents, self.browser.trash().clone());
            (id, self.park(holder))
        } else {
            let slot = self.remove_tab(index)?;
            (slot.id, slot.parked?)
        };
        let layout = self.synced_strip();
        self.strip.hover(&layout, None);
        Some(CarriedTab {
            id,
            tab: Box::new(tab),
        })
    }

    pub fn release_tab(&mut self, id: u64) -> Option<CarriedTab> {
        let index = self.tabs.iter().position(|slot| slot.id == id)?;
        self.take_tab(index)
    }

    pub fn adopt_tab(&mut self, carried: CarriedTab, index: usize) {
        let CarriedTab { id, tab } = carried;
        self.show_parked(*tab, |view, parked| {
            if let Some(slot) = view.tabs.get_mut(view.active_tab) {
                slot.parked = Some(parked);
            }
            view.active_tab = index.min(view.tabs.len());
            view.tabs
                .insert(view.active_tab, TabSlot { id, parked: None });
        });
        let layout = self.synced_strip();
        self.strip.hover(&layout, None);
    }

    pub fn clear_tabs(&mut self) {
        self.tabs.clear();
        self.active_tab = 0;
    }

    pub(crate) fn tab_location(&self, index: usize) -> Option<Location> {
        match &self.tabs.get(index)?.parked {
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

    pub fn tab_layout(&self) -> TabStripLayout {
        let sheet = self.sheet_frame();
        let content = self.content_frame();
        TabStripLayout::new(
            self.tabs.len(),
            content.x + content.width,
            content.x + TAB_INSET,
            TAB_BAR_HEIGHT,
        )
        .below(sheet.y)
    }

    fn synced_strip(&mut self) -> TabStripLayout {
        let ids = self.tabs.iter().map(|slot| slot.id).collect::<Vec<u64>>();
        self.strip.sync(&ids);
        self.tab_layout()
    }

    fn tab_action(&mut self, action: TabAction) {
        match action {
            TabAction::Select(index) => self.select_tab(index),
            TabAction::Close(index) => {
                self.close_tab(index);
            }
            TabAction::NewTab => self.open_tab(self.browser.root().clone()),
            TabAction::Moved { from, to } => {
                move_tab(&mut self.tabs, &mut self.active_tab, from, to);
            }
            TabAction::Detach { index, grab } => {
                self.press = None;
                self.requests.push(Request::DetachTab { index, grab });
            }
            TabAction::Menu { .. } => {}
        }
    }

    pub(crate) fn tab_bar_down(&mut self, pointer: &Pointer) -> bool {
        if !self.shows_tab_bar() {
            return false;
        }
        let layout = self.synced_strip();
        let action = self.strip.press(&layout, pointer);
        if !self.strip.is_engaged() {
            return false;
        }
        self.pending_rename = None;
        self.press = Some(Press::Tabs);
        if let Some(action) = action {
            self.tab_action(action);
        }
        true
    }

    pub(crate) fn tab_bar_moved(&mut self, at: Point) -> bool {
        let layout = self.synced_strip();
        let now = frame_time().on_screen();
        let reduce_motion = self.ui.context().reduce_motion;
        let action = self.strip.moved(&layout, at, now, reduce_motion);
        let moved = action.is_some() || self.strip.is_dragging();
        if let Some(action) = action {
            self.tab_action(action);
        }
        moved
    }

    pub(crate) fn tab_bar_up(&mut self, at: Point) -> bool {
        let layout = self.synced_strip();
        let now = frame_time().on_screen();
        let reduce_motion = self.ui.context().reduce_motion;
        let action = self.strip.released(&layout, at, now, reduce_motion);
        if let Some(action) = action {
            self.tab_action(action);
        }
        true
    }

    pub(crate) fn tab_bar_hover(&mut self, at: Option<Point>) -> bool {
        let layout = self.synced_strip();
        self.strip
            .hover(&layout, at.filter(|_| self.tabs.len() > 1))
    }

    pub(crate) fn tab_bar_cancel(&mut self) {
        let layout = self.tab_layout();
        let now = frame_time().on_screen();
        let reduce_motion = self.ui.context().reduce_motion;
        self.strip.cancel(&layout, now, reduce_motion);
    }

    pub(crate) fn tab_bar_animating(&self) -> bool {
        self.strip.animating(frame_time().on_screen())
    }

    pub fn tab_drop_height(&self) -> f32 {
        TOOLBAR_HEIGHT + self.tab_bar_height()
    }

    pub fn tab_dropped(&mut self, drop: TabDrop) {
        self.requests.push(Request::TabDrop(drop));
    }

    pub fn tab_grab_point(&self, grab: Point) -> Point {
        let content = self.content_frame();
        Point::new(
            content.x + TAB_INSET + grab.x,
            self.sheet_frame().y + grab.y,
        )
    }

    pub fn slide_tab_to(&mut self, id: u64, x: f32) {
        let Some(index) = self.tabs.iter().position(|slot| slot.id == id) else {
            return;
        };
        let layout = self.synced_strip();
        let now = frame_time().on_screen();
        let reduce_motion = self.ui.context().reduce_motion;
        if let Some(action) = self.strip.slide_to(&layout, index, x, now, reduce_motion) {
            self.tab_action(action);
        }
    }

    pub(crate) fn paint_tab_bar(&mut self, items: &mut Vec<Item>) {
        if !self.shows_tab_bar() {
            return;
        }
        let layout = self.synced_strip();
        let titles = self.tab_titles();
        let labels = titles
            .iter()
            .map(|title| TabLabel {
                title,
                badge: false,
            })
            .collect::<Vec<_>>();
        let shown = TabsShown {
            labels: &labels,
            active: self.active_tab,
            focused: self.window_active,
            palette: &self.palette,
        };
        let now = frame_time().on_screen();
        items.extend(
            self.strip
                .paint(&layout, &shown, &mut self.ui.context().text, now),
        );
        for (index, tab) in layout.tabs.iter().enumerate() {
            self.hits.push((*tab, Hit::Tab(index)));
            self.hits
                .push((TabStripLayout::close_button(*tab), Hit::CloseTab(index)));
        }
    }
}
