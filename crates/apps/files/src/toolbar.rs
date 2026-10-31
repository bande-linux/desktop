use bande_toolkit::ui::{
    BUTTON_HEIGHT, Button, SEGMENTED_HEIGHT, SearchField, SegmentedControl, TEXT_FIELD_HEIGHT,
    WidgetId,
};
use bande_toolkit::{Header, Rect, WINDOW_HEADER_HEIGHT, window_controls_area};

use crate::L10N;
use crate::controls::{Signal, sender};
use crate::view::FilesView;
use crate::views::ViewMode;

pub const TOOLBAR_HEIGHT: f32 = WINDOW_HEADER_HEIGHT as f32;
const CONTROLS_GAP: f32 = 16.0;
const TITLE_GAP: f32 = 12.0;
const TRAILING_INSET: f32 = 12.0;
const ITEM_GAP: f32 = 12.0;
const SEARCH_WIDTH: f32 = 240.0;
const MIN_SEARCH_WIDTH: f32 = 140.0;
const SYMBOL_PIXELS: u32 = 32;
const SYMBOL_SEGMENT: f32 = 32.0;
const MODES: [ViewMode; 3] = [ViewMode::Icons, ViewMode::List, ViewMode::Columns];

pub(crate) struct Toolbar {
    pub(crate) back: WidgetId,
    pub(crate) forward: WidgetId,
    pub(crate) view: WidgetId,
    pub(crate) search: WidgetId,
    enabled: (bool, bool),
}

impl Default for Toolbar {
    fn default() -> Self {
        Self {
            back: WidgetId::next(),
            forward: WidgetId::next(),
            view: WidgetId::next(),
            search: WidgetId::next(),
            enabled: (true, true),
        }
    }
}

fn centred(height: f32) -> f32 {
    ((TOOLBAR_HEIGHT - height) / 2.0).round()
}

impl FilesView {
    fn symbol_button(&mut self, icon: &str, label: &str, signal: Signal) -> Button {
        let send = sender(&self.signals);
        let click = move || send(signal.clone());
        match self.icons.first(&[icon], SYMBOL_PIXELS) {
            Some(image) => Button::symbol(image, label, click),
            None => Button::new(label, click),
        }
    }

    pub(crate) fn insert_toolbar(&mut self) {
        let back = self.symbol_button("go-previous-symbolic", &L10N.tr("menu-back"), Signal::Back);
        let forward = self.symbol_button(
            "go-next-symbolic",
            &L10N.tr("menu-forward"),
            Signal::Forward,
        );
        self.toolbar.back = self.insert(back);
        self.toolbar.forward = self.insert(forward);
        let view = self.view_control_widget();
        self.toolbar.view = self.insert(view);
        let search = self.search_field_widget();
        self.toolbar.search = self.insert(search);
        self.refresh_navigation();
    }

    pub(crate) fn search_field_widget(&self) -> SearchField {
        let send = sender(&self.signals);
        SearchField::new(&L10N.tr("search-placeholder"), "", move |text| {
            send(Signal::Search(text.to_owned()));
        })
    }

    fn view_control_widget(&mut self) -> SegmentedControl {
        let send = sender(&self.signals);
        let choices = [
            ("view-grid-symbolic", "view-icons"),
            ("view-list-symbolic", "view-list"),
            ("view-column-symbolic", "view-columns"),
        ];
        let symbols = choices
            .iter()
            .map(|(icon, title)| {
                self.icons
                    .first(&[icon], SYMBOL_PIXELS)
                    .map(|image| (image, L10N.tr(title)))
            })
            .collect::<Option<Vec<_>>>();
        let selected = MODES.iter().position(|mode| *mode == self.mode);
        let label = L10N.tr("view-label");
        let select = move |index| send(Signal::View(index));
        match symbols {
            Some(symbols) => SegmentedControl::symbols(&label, symbols, selected, select),
            None => {
                let titles = choices.map(|(_, title)| L10N.tr(title));
                SegmentedControl::new(&label, &titles, selected, select)
            }
        }
    }

    pub(crate) fn view_chosen(&mut self, index: usize) {
        if let Some(mode) = MODES.get(index).copied() {
            self.set_mode(mode);
        }
    }

    pub(crate) fn refresh_view_control(&mut self) {
        let enabled = self.searching.is_none();
        if let Some(wanted) = MODES.iter().position(|mode| *mode == self.mode) {
            self.ui
                .update_widget(self.toolbar.view, |control: &mut SegmentedControl, cx| {
                    control.set_selected(cx, wanted);
                    control.set_enabled(enabled);
                });
        }
    }

    pub(crate) fn refresh_navigation(&mut self) {
        let enabled = (
            self.searching.is_some() || self.browser.can_go_back(),
            self.searching.is_none() && self.browser.can_go_forward(),
        );
        if enabled == self.toolbar.enabled {
            return;
        }
        self.toolbar.enabled = enabled;
        for (id, on) in [
            (self.toolbar.back, enabled.0),
            (self.toolbar.forward, enabled.1),
        ] {
            if let Some(button) = self.ui.widget_as::<Button>(id) {
                button.set_enabled(on);
            }
        }
    }

    pub(crate) fn reset_search_field(&mut self) {
        let field = self.search_field_widget();
        self.toolbar.search = self.replace_widget(self.toolbar.search, field);
    }

    pub(crate) fn toolbar_frames(&self) -> [(WidgetId, Rect); 4] {
        let back_x = window_controls_area(TOOLBAR_HEIGHT).width + CONTROLS_GAP;
        let button_y = centred(BUTTON_HEIGHT);
        let right = self.size.0 - TRAILING_INSET;
        let view_width = MODES.len() as f32 * SYMBOL_SEGMENT;
        let room = right - (back_x + 2.0 * BUTTON_HEIGHT + 120.0 + view_width + ITEM_GAP);
        let search_width = SEARCH_WIDTH.min(room).max(MIN_SEARCH_WIDTH);
        let search_x = right - search_width;
        [
            (
                self.toolbar.back,
                Rect::new(back_x, button_y, BUTTON_HEIGHT, BUTTON_HEIGHT),
            ),
            (
                self.toolbar.forward,
                Rect::new(
                    back_x + BUTTON_HEIGHT,
                    button_y,
                    BUTTON_HEIGHT,
                    BUTTON_HEIGHT,
                ),
            ),
            (
                self.toolbar.view,
                Rect::new(
                    search_x - ITEM_GAP - view_width,
                    centred(SEGMENTED_HEIGHT),
                    view_width,
                    SEGMENTED_HEIGHT,
                ),
            ),
            (
                self.toolbar.search,
                Rect::new(
                    search_x,
                    centred(TEXT_FIELD_HEIGHT),
                    search_width,
                    TEXT_FIELD_HEIGHT,
                ),
            ),
        ]
    }

    pub fn header(&self) -> Header {
        let frames = self.toolbar_frames();
        let forward = frames[1].1;
        Header {
            title_x: forward.x + forward.width + TITLE_GAP,
            items: frames.iter().map(|(_, frame)| *frame).collect(),
            tabs: Vec::new(),
        }
    }
}
