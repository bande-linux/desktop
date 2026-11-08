use std::path::Path;

use bande_fs::format_size;
use bande_l10n::{tr, trn};
use bande_settings_panes::{PaneId, SettingId};
use bande_toolkit::Spacing;
use bande_toolkit::ui::{Axis, Button, Label, SearchField, Stack, Widget};

use crate::login::App;
use crate::pages::{Widgets, boxed, muted_row, submit};
use crate::software::SoftwareState;
use crate::storage::Category;
use crate::view::{Anchor, Intent, Picker, SettingsView, intent};
use crate::widgets::{Row, Segment, Text, Tone, UsageBar};

pub(crate) fn info_row(label: &str, value: &str) -> Box<dyn Widget> {
    boxed(Row::new(
        Text::new(label, Tone::Primary),
        Some(boxed(Label::new(value))),
    ))
}

fn category_name(category: Category) -> String {
    match category {
        Category::Apps => tr!("Apps"),
        Category::Documents => tr!("Documents"),
        Category::Trash => tr!("Trash"),
        Category::System => tr!("System"),
    }
}

fn available_of(available: u64, capacity: u64) -> String {
    tr!(
        "{available} available of {capacity}",
        available = format_size(available),
        capacity = format_size(capacity)
    )
}

fn shown_path(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) => Path::new("~").join(rest).to_string_lossy().into_owned(),
        None => path.to_string_lossy().into_owned(),
    }
}

impl SettingsView {
    pub(crate) fn pane_summary(&self, pane: PaneId) -> Option<String> {
        match pane {
            PaneId::SoftwareUpdate => match &self.software {
                SoftwareState::Available(packages) => Some(trn!(
                    "1 update is available",
                    "{count} updates are available",
                    count = packages.len()
                )),
                SoftwareState::UpToDate => Some(tr!("Your computer is up to date")),
                SoftwareState::Unavailable => {
                    Some(tr!("Updates aren't available on this computer"))
                }
                state => state.pending(),
            },
            PaneId::Storage => self
                .about
                .as_ref()
                .map(|about| available_of(about.available, about.capacity)),
            PaneId::Diagnostics => Some(match self.diagnostics.reports.len() {
                0 => tr!(context = "count", "None"),
                count => trn!("One report", "{count} reports", count = count),
            }),
            PaneId::NightLight => self.night_light_summary(),
            PaneId::Camera
            | PaneId::Microphone
            | PaneId::Screenshots
            | PaneId::ScreenSharing
            | PaneId::Location => self.grant_summary(pane),
            _ => None,
        }
    }

    pub(crate) fn back_button(&mut self, parent: PaneId) -> Option<Box<dyn Widget>> {
        let name = self
            .panes
            .iter()
            .find(|pane| pane.id == parent)?
            .name
            .clone();
        let mut back = intent(&self.intents, |()| Intent::Back);
        let button = Button::new(&tr!("‹ {pane}", pane = name), move || back(()))
            .tooltip(&tr!("Go back (Alt+Left)"));
        self.anchors.push((Anchor::Back, button.id()));
        Some(boxed(Stack::new(
            Axis::Horizontal,
            0.0,
            0.0,
            vec![boxed(button)],
        )))
    }

    pub(crate) fn about_rows(&mut self) -> Widgets {
        let Some(about) = self.about.clone() else {
            return Vec::new();
        };
        let mut rows = vec![
            info_row(&tr!("Model"), &about.model),
            info_row(&tr!("Chip"), &about.chip),
            info_row(&tr!("Memory"), &format_size(about.memory)),
            info_row(&tr!("Bande"), &about.bande),
            info_row(&tr!("System"), &about.system),
        ];
        if let Some(serial) = &about.serial {
            rows.push(info_row(&tr!("Serial number"), serial));
        }
        rows
    }

    pub(crate) fn storage_rows(&mut self) -> Widgets {
        let Some(storage) = self.storage.clone() else {
            return vec![muted_row(&tr!("Calculating…"))];
        };
        let summary = available_of(storage.available, storage.capacity);
        let segments = match &storage.categories {
            Some(categories) => categories
                .iter()
                .map(|(category, bytes)| Segment {
                    label: tr!(
                        "{category} {size}",
                        category = category_name(*category),
                        size = format_size(*bytes)
                    ),
                    bytes: *bytes,
                })
                .collect(),
            None => vec![Segment {
                label: tr!("Calculating…"),
                bytes: storage.used(),
            }],
        };
        let mut rows = vec![
            boxed(
                Row::new(Text::new(&tr!("Startup disk"), Tone::Primary), None)
                    .detail(Text::new(&summary, Tone::Muted).caption()),
            ),
            boxed(UsageBar::new(&summary, segments, storage.capacity)),
        ];
        rows.push(self.trash_row(storage.trash_items));
        rows
    }

    fn trash_row(&mut self, items: usize) -> Box<dyn Widget> {
        let detail = if items == 0 {
            tr!("Empty")
        } else {
            trn!("1 item", "{count} items", count = items)
        };
        if self.confirming_empty_trash && items > 0 {
            let mut cancel = intent(&self.intents, |()| Intent::AskEmptyTrash(false));
            let mut empty = intent(&self.intents, |()| Intent::EmptyTrash);
            let erase = Button::new(&tr!("Empty Trash"), move || empty(()));
            self.anchors
                .push((Anchor::Item("empty-trash".to_owned()), erase.id()));
            let buttons = Stack::new(
                Axis::Horizontal,
                Spacing::S as f32,
                0.0,
                vec![
                    boxed(Button::new(&tr!("Cancel"), move || cancel(())).cancel()),
                    boxed(erase.primary()),
                ],
            );
            return boxed(
                Row::new(
                    Text::new(
                        &trn!(
                            "Erase the item in the Trash?",
                            "Erase all {count} items in the Trash?",
                            count = items
                        ),
                        Tone::Primary,
                    ),
                    Some(boxed(buttons)),
                )
                .detail(Text::new(&tr!("This cannot be undone."), Tone::Muted).caption())
                .highlighted(true),
            );
        }
        let control = (items > 0).then(|| {
            let mut ask = intent(&self.intents, |()| Intent::AskEmptyTrash(true));
            let button = Button::new(&tr!("Empty Trash…"), move || ask(()));
            self.anchors
                .push((Anchor::Item("trash".to_owned()), button.id()));
            boxed(button)
        });
        boxed(
            Row::new(Text::new(&tr!("Trash"), Tone::Primary), control)
                .detail(Text::new(&detail, Tone::Muted).caption()),
        )
    }

    pub(crate) fn login_rows(&mut self) -> Widgets {
        let Some(login) = self.login.clone() else {
            return Vec::new();
        };
        let mut rows: Widgets = Vec::new();
        if login.items.is_empty() {
            rows.push(muted_row(&tr!("No apps open when you log in")));
        }
        for item in &login.items {
            let mut remove = intent(&self.intents, Intent::RemoveLoginItem);
            let id = item.id.clone();
            let button = Button::new(&tr!("Remove"), move || remove(id.clone()))
                .tooltip(&tr!("Stop opening this app when you log in"));
            let row = Row::new(Text::new(&item.name, Tone::Primary), Some(boxed(button)));
            rows.push(self.item_row(format!("login:{}", item.id), row));
        }
        let picking = self
            .picking
            .clone()
            .filter(|(picker, _)| *picker == Picker::LoginApp);
        let Some((_, query)) = picking else {
            let mut add = intent(&self.intents, |()| Intent::Pick(Some(Picker::LoginApp)));
            let button = Button::new(&tr!("Add App…"), move || add(()));
            self.anchors
                .push((Anchor::Setting(SettingId::LoginItems, None), button.id()));
            rows.push(boxed(Row::new(
                Text::new(&tr!("These apps open each time you log in"), Tone::Muted).caption(),
                Some(boxed(button)),
            )));
            return rows;
        };
        rows.extend(self.app_search(&tr!("Add App…"), &query, &login.apps));
        rows
    }

    pub(crate) fn app_search(&mut self, title: &str, query: &str, apps: &[App]) -> Widgets {
        let field = SearchField::new(
            &tr!("Search apps"),
            query,
            submit(&self.intents, Intent::PickQuery),
        );
        self.anchors.push((Anchor::Picker, field.id()));
        let mut cancel = intent(&self.intents, |()| Intent::Pick(None));
        let mut rows = vec![boxed(Row::new(
            Text::new(title, Tone::Primary),
            Some(boxed(Stack::new(
                Axis::Horizontal,
                Spacing::S as f32,
                0.0,
                vec![
                    boxed(field),
                    boxed(Button::new(&tr!("Cancel"), move || cancel(())).cancel()),
                ],
            ))),
        ))];
        let results = self.pick_results();
        if results.is_empty() && !query.trim().is_empty() {
            rows.push(muted_row(&tr!("No app matches")));
        }
        for id in results {
            let Some(app) = apps.iter().find(|app| app.id == id) else {
                continue;
            };
            let mut choose = intent(&self.intents, Intent::Picked);
            let chosen = id.clone();
            let row = Row::new(Text::new(&app.name, Tone::Primary), None)
                .on_activate(move || choose(chosen.clone()));
            rows.push(self.item_row(format!("add:{id}"), row));
        }
        rows
    }

    pub(crate) fn large_file_rows(&mut self) -> Widgets {
        let Some(storage) = self.storage.clone() else {
            return Vec::new();
        };
        if storage.categories.is_none() {
            return vec![muted_row(&tr!("Calculating…"))];
        }
        if storage.largest.is_empty() {
            return vec![muted_row(&tr!("No large files in your home folder"))];
        }
        let home = std::env::home_dir();
        storage
            .largest
            .into_iter()
            .map(|(path, bytes)| {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let folder = path
                    .parent()
                    .map(|folder| shown_path(folder, home.as_deref()))
                    .unwrap_or_default();
                let mut controls = vec![boxed(Label::new(&format_size(bytes)))];
                if self.files_installed {
                    let mut reveal = intent(&self.intents, Intent::Reveal);
                    let target = path.clone();
                    controls.push(boxed(Button::new(&tr!("Show in Files"), move || {
                        reveal(target.clone())
                    })));
                }
                let controls = Stack::new(Axis::Horizontal, Spacing::M as f32, 0.0, controls);
                let row = Row::new(Text::new(&name, Tone::Primary), Some(boxed(controls)))
                    .detail(Text::new(&folder, Tone::Muted).caption());
                self.item_row(path.to_string_lossy().into_owned(), row)
            })
            .collect()
    }
}
