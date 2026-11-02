use bande_toolkit::ui::{Button, Table, WidgetId};
use bande_toolkit::{ControlSize, Item, Point, Rect, TextAlign, TextRole, TextStyle};

use crate::L10N;
use crate::browser::Location;
use crate::controls::{Signal, sender};
use crate::tabs::location_title;
use crate::view::FilesView;
use crate::views::ViewMode;

const GAP: f32 = 8.0;
const MARGIN: f32 = 16.0;
const BUTTON_PADDING: f32 = 16.0;

impl FilesView {
    pub(crate) fn empty_message(&self, index: usize) -> Option<String> {
        let column = self.browser.columns().get(index)?;
        if !column.entries.is_empty() || column.is_listing() {
            return None;
        }
        Some(match &column.location {
            Location::Folder(_) if column.denied => L10N.tr_args(
                "no-permission",
                [("folder", location_title(&column.location).into())],
            ),
            Location::Folder(_) => L10N.tr("empty-folder"),
            Location::Trash => L10N.tr("trash-empty"),
            Location::Recents => L10N.tr("recents-empty"),
            Location::Search(scope) => {
                let searching = self.searching.as_ref().filter(|shown| shown.answered)?;
                L10N.tr_args(
                    "no-results",
                    [
                        ("text", searching.text.clone().into()),
                        (
                            "folder",
                            location_title(&Location::Folder(scope.clone())).into(),
                        ),
                    ],
                )
            }
        })
    }

    pub(crate) fn wants_search_home(&self) -> bool {
        self.searching
            .as_ref()
            .is_some_and(|searching| !searching.everywhere)
            && self.empty_message(0).is_some()
    }

    pub(crate) fn sync_search_home(&mut self) {
        let wanted = self.wants_search_home();
        let shown = self.search_home_button.is_some();
        if wanted == shown {
            return;
        }
        match self.search_home_button.take() {
            Some(button) => self.remove(button),
            None => {
                let send = sender(&self.signals);
                let button = Button::new(&L10N.tr("search-home"), move || {
                    send(Signal::Scope(true));
                })
                .quiet();
                self.search_home_button = Some(self.insert(button));
            }
        }
    }

    pub(crate) fn empty_area(&self) -> Rect {
        let content = self.content_frame();
        match self.mode {
            ViewMode::List => Rect::new(
                content.x,
                content.y + Table::HEADER_HEIGHT,
                content.width,
                (content.height - Table::HEADER_HEIGHT).max(0.0),
            ),
            _ => content,
        }
    }

    pub(crate) fn search_home_frame(&mut self, area: Rect) -> Option<(WidgetId, Rect)> {
        let button = self.search_home_button?;
        let height = self.ui.context().control_height(ControlSize::Regular);
        let ink = self.palette.ink;
        let label = self.text().paragraph(
            &L10N.tr("search-home"),
            TextStyle::role(TextRole::Body, ink),
            None,
            TextAlign::Start,
        );
        let width = (label.width() + 2.0 * BUTTON_PADDING).ceil();
        let centre = area.y + (area.height / 2.0).round();
        Some((
            button,
            Rect::new(
                area.x + ((area.width - width) / 2.0).round(),
                centre + GAP / 2.0,
                width,
                height,
            ),
        ))
    }

    pub(crate) fn paint_empty(&mut self, index: usize, area: Rect, items: &mut Vec<Item>) {
        let Some(message) = self.empty_message(index) else {
            return;
        };
        let ink = self.palette.ink3;
        let width = (area.width - 2.0 * MARGIN).max(0.0);
        let paragraph = self.text().paragraph(
            &message,
            TextStyle::role(TextRole::Body, ink),
            Some(width),
            TextAlign::Center,
        );
        let centre = area.y + (area.height / 2.0).round();
        let y = match index == 0 && self.search_home_button.is_some() {
            true => centre - GAP / 2.0 - paragraph.height(),
            false => centre - (paragraph.height() / 2.0).round(),
        };
        items.push(Item::Text {
            origin: Point::new(area.x + MARGIN, y.round()),
            paragraph,
        });
    }
}
