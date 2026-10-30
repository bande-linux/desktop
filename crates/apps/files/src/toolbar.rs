use unbound_toolkit::ui::{
    SEGMENTED_HEIGHT, SearchField, SegmentedControl, TEXT_FIELD_HEIGHT, WidgetId,
};
use unbound_toolkit::{Item, Point, Rect, Shape, TextAlign, TextStyle};

use crate::L10N;
use crate::controls::{Signal, sender};
use crate::view::{FilesView, Hit, SIDEBAR_WIDTH};
use crate::views::ViewMode;

pub const TOOLBAR_HEIGHT: f32 = 40.0;
const BUTTON: f32 = 28.0;
const SEARCH_WIDTH: f32 = 220.0;
const VIEW_CONTROL_WIDTH: f32 = 216.0;
const MODES: [ViewMode; 3] = [ViewMode::Icons, ViewMode::List, ViewMode::Columns];

impl FilesView {
    pub(crate) fn search_field_widget(&self) -> SearchField {
        let send = sender(&self.signals);
        SearchField::new(&L10N.tr("search-placeholder"), "", move |text| {
            send(Signal::Search(text.to_owned()));
        })
    }

    pub(crate) fn view_control_widget(&self) -> SegmentedControl {
        let send = sender(&self.signals);
        let titles = [
            L10N.tr("view-icons"),
            L10N.tr("view-list"),
            L10N.tr("view-columns"),
        ];
        let selected = MODES.iter().position(|mode| *mode == self.mode);
        SegmentedControl::new(&L10N.tr("view-label"), &titles, selected, move |index| {
            send(Signal::View(index));
        })
    }

    pub(crate) fn view_chosen(&mut self, index: usize) {
        if let Some(mode) = MODES.get(index).copied() {
            self.set_mode(mode);
        }
    }

    pub(crate) fn refresh_view_control(&mut self) {
        let wanted = MODES.iter().position(|mode| *mode == self.mode);
        let shown = self
            .ui
            .widget_ref::<SegmentedControl>(self.view_control)
            .and_then(SegmentedControl::selected);
        if shown == wanted {
            return;
        }
        let control = self.view_control_widget();
        self.view_control = self.replace_widget(self.view_control, control);
    }

    pub(crate) fn reset_search_field(&mut self) {
        let field = self.search_field_widget();
        self.search_field = self.replace_widget(self.search_field, field);
    }

    pub(crate) fn toolbar_frames(&self) -> [(WidgetId, Rect); 2] {
        let right = self.size.0;
        let search_width = SEARCH_WIDTH.min((right - SIDEBAR_WIDTH - 300.0).max(120.0));
        [
            (
                self.view_control,
                Rect::new(
                    SIDEBAR_WIDTH + 86.0,
                    ((TOOLBAR_HEIGHT - SEGMENTED_HEIGHT) / 2.0).round(),
                    VIEW_CONTROL_WIDTH,
                    SEGMENTED_HEIGHT,
                ),
            ),
            (
                self.search_field,
                Rect::new(
                    right - search_width - 10.0,
                    ((TOOLBAR_HEIGHT - TEXT_FIELD_HEIGHT) / 2.0).round(),
                    search_width,
                    TEXT_FIELD_HEIGHT,
                ),
            ),
        ]
    }

    fn arrow(&mut self, symbol: &str, origin: Point, enabled: bool, hit: Hit) -> Vec<Item> {
        let area = Rect::new(origin.x, origin.y, BUTTON, BUTTON);
        let mut items = Vec::new();
        if enabled && self.hover.as_ref() == Some(&hit) {
            items.push(Item::Fill {
                shape: Shape::rounded(area, 6.0),
                paint: self.palette.ink2.with_alpha(30).into(),
            });
        }
        let color = match enabled {
            true => self.palette.ink,
            false => self.palette.ink2.with_alpha(90),
        };
        let paragraph = self.text().paragraph(
            symbol,
            TextStyle::new(22.0, color),
            Some(BUTTON),
            TextAlign::Center,
        );
        items.push(Item::Text {
            origin: Point::new(
                area.x,
                area.y + ((BUTTON - paragraph.height()) / 2.0).round() - 1.0,
            ),
            paragraph,
        });
        if enabled {
            self.hits.push((area, hit));
        }
        items
    }

    pub(crate) fn paint_toolbar(&mut self, items: &mut Vec<Item>) {
        let bar = Rect::new(
            SIDEBAR_WIDTH,
            0.0,
            (self.size.0 - SIDEBAR_WIDTH).max(0.0),
            TOOLBAR_HEIGHT,
        );
        items.push(Item::Fill {
            shape: Shape::rect(bar),
            paint: self.palette.sheet.into(),
        });
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(bar.x, bar.height - 1.0, bar.width, 1.0)),
            paint: self.palette.hairline.into(),
        });
        let top = ((TOOLBAR_HEIGHT - BUTTON) / 2.0).round();
        let back = self.browser.can_go_back();
        let forward = self.browser.can_go_forward();
        let mut arrows = self.arrow("\u{2039}", Point::new(bar.x + 10.0, top), back, Hit::Back);
        arrows.extend(self.arrow(
            "\u{203A}",
            Point::new(bar.x + 42.0, top),
            forward,
            Hit::Forward,
        ));
        items.extend(arrows);
    }
}
