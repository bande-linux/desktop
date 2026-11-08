use std::path::PathBuf;

use bande_l10n::tr;

use crate::controls::{Signal, sender};
use crate::prompt::{Kind, Prompt};
use crate::view::{FilesView, Request, Sheet};
use bande_toolkit::ui::{
    Axis, Bounded, Cell, CheckState, Checkbox, Column, Row, SearchField, Stack, Table, Widget,
    WidgetId,
};
use bande_xdg::DesktopEntry;

const LIST_HEIGHT: f32 = 240.0;
const SPACING: f32 = 10.0;
const ICON_PIXELS: u32 = 32;
const FALLBACK_ICON: &str = "application-x-executable";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWith {
    pub files: Vec<PathBuf>,
    pub mime: String,
    pub always: bool,
    pub(crate) table: WidgetId,
}

fn matches(app: &DesktopEntry, search: &str) -> bool {
    search.is_empty()
        || app.name.to_lowercase().contains(search)
        || app
            .generic_name
            .as_ref()
            .is_some_and(|name| name.to_lowercase().contains(search))
}

impl FilesView {
    fn open_with_apps(&self, mime: &str) -> Vec<DesktopEntry> {
        let recommended = self.apps.apps_for(mime);
        let others = self
            .apps
            .openers()
            .into_iter()
            .filter(|app| !recommended.iter().any(|known| known.id == app.id));
        recommended.iter().copied().chain(others).cloned().collect()
    }

    fn open_with_rows(&mut self, mime: &str, search: &str) -> Vec<Row> {
        let search = search.trim().to_lowercase();
        self.open_with_apps(mime)
            .into_iter()
            .filter(|app| matches(app, &search))
            .map(|app| {
                let icon = self.icons.first(
                    &[app.icon.as_deref().unwrap_or(FALLBACK_ICON), FALLBACK_ICON],
                    ICON_PIXELS,
                );
                Row::new(
                    app.id.clone(),
                    vec![Cell::new(app.name.as_str()).icon(icon)],
                )
            })
            .collect()
    }

    pub(crate) fn ask_open_with(&mut self, files: Vec<PathBuf>) {
        let Some(first) = files.first() else {
            return;
        };
        let mime = self.apps.mime_for(first);
        let rows = self.open_with_rows(&mime, "");
        let first_key = rows.first().map(|row| row.key.clone());
        let (activate, search, always) = (
            sender(&self.signals),
            sender(&self.signals),
            sender(&self.signals),
        );
        let mut table = Table::new(vec![Column::new(&tr!("Name"), 300.0).flexible()])
            .without_header()
            .on_activate(move |_| activate(Signal::OpenWithActivate));
        table.set_rows(rows);
        table.select(&first_key.into_iter().collect::<Vec<_>>());
        let table_id = table.id();
        let field = SearchField::new(&tr!("Search"), "", move |text| {
            search(Signal::OpenWithSearch(text.to_owned()));
        });
        let field_id = field.id();
        let checkbox = Checkbox::new(&tr!("Always Open With"), CheckState::Off, move |on| {
            always(Signal::OpenWithAlways(on));
        });
        let accessory = Stack::new(
            Axis::Vertical,
            SPACING,
            0.0,
            vec![
                Box::new(field),
                Box::new(Bounded::new(table, LIST_HEIGHT)),
                Box::new(checkbox),
            ],
        );
        let title = match files.as_slice() {
            [file] => tr!(
                "Choose an app to open “{name}”:",
                name = file
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
            _ => tr!("Choose an app to open {count} items:", count = files.len()),
        };
        let prompt = Prompt::new(&title, "", self.choice())
            .accessory(accessory)
            .button(&tr!("Cancel"), "cancel", Kind::Cancel)
            .button(&tr!("Open"), "open", Kind::Primary);
        let cancel = prompt.cancel_button();
        self.show_sheet(
            prompt,
            Sheet::OpenWith(OpenWith {
                files,
                mime,
                always: false,
                table: table_id,
            }),
            cancel,
        );
        self.focus(field_id);
    }

    pub(crate) fn filter_open_with(&mut self, search: &str) {
        let Some(Sheet::OpenWith(open_with)) = &self.sheet else {
            return;
        };
        let (mime, table) = (open_with.mime.clone(), open_with.table);
        let rows = self.open_with_rows(&mime, search);
        let first_key = rows.first().map(|row| row.key.clone());
        if let Some(table) = self.ui.widget_as::<Table>(table) {
            let kept = table.selection();
            table.set_rows(rows);
            if table.selection().is_empty() || kept.is_empty() {
                table.select(&first_key.into_iter().collect::<Vec<_>>());
            }
        }
    }

    pub(crate) fn open_with_chosen_app(&mut self, open_with: OpenWith) {
        let chosen = self
            .ui
            .widget_ref::<Table>(open_with.table)
            .and_then(|table| table.selection().into_iter().next());
        let Some(app) = chosen.and_then(|id| {
            self.apps
                .openers()
                .into_iter()
                .chain(self.apps.apps_for(&open_with.mime))
                .find(|app| app.id == id)
                .cloned()
        }) else {
            return;
        };
        if open_with.always
            && let Err(error) = self.apps.set_default(&open_with.mime, &app)
        {
            self.show_notice(tr!(
                "The default app couldn’t be changed: {error}",
                error = error.to_string()
            ));
        }
        self.requests.push(Request::Open {
            app: Box::new(app),
            files: open_with.files,
        });
    }
}
