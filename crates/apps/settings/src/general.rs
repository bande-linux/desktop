use std::path::Path;

use unbound_fs::format_size;
use unbound_settings_panes::{PaneId, SettingId, Value, duration_label};
use unbound_toolkit::Spacing;
use unbound_toolkit::ui::{Axis, Button, Label, SearchField, Stack, Widget};

use crate::L10N;
use crate::pages::{Widgets, boxed, muted_row, submit};
use crate::software::SoftwareState;
use crate::storage::Category;
use crate::view::{Anchor, Intent, Picker, SettingsView, intent};
use crate::widgets::{Row, Segment, Text, Tone, UsageBar};

fn info_row(label: &str, value: &str) -> Box<dyn Widget> {
    boxed(Row::new(
        Text::new(label, Tone::Primary),
        Some(boxed(Label::new(value))),
    ))
}

fn category_name(category: Category) -> String {
    L10N.tr(match category {
        Category::Apps => "storage-apps",
        Category::Documents => "storage-documents",
        Category::Trash => "storage-trash",
        Category::System => "storage-system",
    })
}

fn available_of(available: u64, capacity: u64) -> String {
    L10N.tr_args(
        "available-of",
        [
            ("available", format_size(available).into()),
            ("capacity", format_size(capacity).into()),
        ],
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
                SoftwareState::Available(packages) => {
                    Some(L10N.tr_args("updates-available", [("count", packages.len().into())]))
                }
                SoftwareState::UpToDate => Some(L10N.tr("up-to-date")),
                SoftwareState::Installed { restart: true } => Some(L10N.tr("restart-to-update")),
                _ => None,
            },
            PaneId::Storage => self
                .about
                .as_ref()
                .map(|about| available_of(about.available, about.capacity)),
            PaneId::Power => match self.values.get(&SettingId::LockAfter) {
                Some(Value::Seconds(seconds)) => {
                    Some(L10N.tr_args("lock-summary", [("time", duration_label(*seconds).into())]))
                }
                _ => None,
            },
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
        let button = Button::new(
            &L10N.tr_args("back-to", [("pane", name.into())]),
            move || back(()),
        )
        .tooltip(&L10N.tr("back-tooltip"));
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
            info_row(&L10N.tr("about-model"), &about.model),
            info_row(&L10N.tr("about-chip"), &about.chip),
            info_row(&L10N.tr("about-memory"), &format_size(about.memory)),
            info_row(&L10N.tr("about-unbound"), &about.unbound),
            info_row(&L10N.tr("about-system"), &about.system),
        ];
        if let Some(serial) = &about.serial {
            rows.push(info_row(&L10N.tr("about-serial"), serial));
        }
        rows
    }

    pub(crate) fn storage_rows(&mut self) -> Widgets {
        let Some(storage) = self.storage.clone() else {
            return vec![muted_row(&L10N.tr("calculating"))];
        };
        let summary = available_of(storage.available, storage.capacity);
        let segments = match &storage.categories {
            Some(categories) => categories
                .iter()
                .map(|(category, bytes)| Segment {
                    label: L10N.tr_args(
                        "storage-category",
                        [
                            ("category", category_name(*category).into()),
                            ("size", format_size(*bytes).into()),
                        ],
                    ),
                    bytes: *bytes,
                })
                .collect(),
            None => vec![Segment {
                label: L10N.tr("calculating"),
                bytes: storage.used(),
            }],
        };
        let mut rows = vec![
            boxed(
                Row::new(Text::new(&L10N.tr("startup-disk"), Tone::Primary), None)
                    .detail(Text::new(&summary, Tone::Muted).caption()),
            ),
            boxed(UsageBar::new(&summary, segments, storage.capacity)),
        ];
        rows.push(self.trash_row(storage.trash_items));
        rows
    }

    fn trash_row(&mut self, items: usize) -> Box<dyn Widget> {
        let detail = if items == 0 {
            L10N.tr("trash-empty")
        } else {
            L10N.tr_args("trash-items", [("count", items.into())])
        };
        if self.confirming_empty_trash && items > 0 {
            let mut cancel = intent(&self.intents, |()| Intent::AskEmptyTrash(false));
            let mut empty = intent(&self.intents, |()| Intent::EmptyTrash);
            let erase = Button::new(&L10N.tr("empty-trash"), move || empty(()));
            self.anchors
                .push((Anchor::Item("empty-trash".to_owned()), erase.id()));
            let buttons = Stack::new(
                Axis::Horizontal,
                Spacing::S as f32,
                0.0,
                vec![
                    boxed(Button::new(&L10N.tr("cancel"), move || cancel(())).cancel()),
                    boxed(erase.primary()),
                ],
            );
            return boxed(
                Row::new(
                    Text::new(
                        &L10N.tr_args("empty-trash-question", [("count", items.into())]),
                        Tone::Primary,
                    ),
                    Some(boxed(buttons)),
                )
                .detail(Text::new(&L10N.tr("empty-trash-warning"), Tone::Muted).caption())
                .highlighted(true),
            );
        }
        let control = (items > 0).then(|| {
            let mut ask = intent(&self.intents, |()| Intent::AskEmptyTrash(true));
            let button = Button::new(&L10N.tr("empty-trash-ellipsis"), move || ask(()));
            self.anchors
                .push((Anchor::Item("trash".to_owned()), button.id()));
            boxed(button)
        });
        boxed(
            Row::new(Text::new(&L10N.tr("storage-trash"), Tone::Primary), control)
                .detail(Text::new(&detail, Tone::Muted).caption()),
        )
    }

    pub(crate) fn login_rows(&mut self) -> Widgets {
        let Some(login) = self.login.clone() else {
            return Vec::new();
        };
        let mut rows: Widgets = Vec::new();
        if login.items.is_empty() {
            rows.push(muted_row(&L10N.tr("no-login-items")));
        }
        for item in &login.items {
            let mut remove = intent(&self.intents, Intent::RemoveLoginItem);
            let id = item.id.clone();
            let button = Button::new(&L10N.tr("remove"), move || remove(id.clone()))
                .tooltip(&L10N.tr("remove-login-item-tooltip"));
            let row = Row::new(Text::new(&item.name, Tone::Primary), Some(boxed(button)));
            rows.push(self.item_row(format!("login:{}", item.id), row));
        }
        let picking = self
            .picking
            .clone()
            .filter(|(picker, _)| *picker == Picker::LoginApp);
        let Some((_, query)) = picking else {
            let mut add = intent(&self.intents, |()| Intent::Pick(Some(Picker::LoginApp)));
            let button = Button::new(&L10N.tr("add-login-item"), move || add(()));
            self.anchors
                .push((Anchor::Setting(SettingId::LoginItems, None), button.id()));
            rows.push(boxed(Row::new(
                Text::new(&L10N.tr("login-items-hint"), Tone::Muted).caption(),
                Some(boxed(button)),
            )));
            return rows;
        };
        let field = SearchField::new(
            &L10N.tr("search-apps"),
            &query,
            submit(&self.intents, Intent::PickQuery),
        );
        self.anchors.push((Anchor::Picker, field.id()));
        let mut cancel = intent(&self.intents, |()| Intent::Pick(None));
        rows.push(boxed(Row::new(
            Text::new(&L10N.tr("add-login-item"), Tone::Primary),
            Some(boxed(Stack::new(
                Axis::Horizontal,
                Spacing::S as f32,
                0.0,
                vec![
                    boxed(field),
                    boxed(Button::new(&L10N.tr("cancel"), move || cancel(())).cancel()),
                ],
            ))),
        )));
        let results = self.pick_results();
        if results.is_empty() && !query.trim().is_empty() {
            rows.push(muted_row(&L10N.tr("no-apps")));
        }
        for id in results {
            let Some(app) = login.apps.iter().find(|app| app.id == id) else {
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
            return vec![muted_row(&L10N.tr("calculating"))];
        }
        if storage.largest.is_empty() {
            return vec![muted_row(&L10N.tr("no-large-files"))];
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
                let mut reveal = intent(&self.intents, Intent::Reveal);
                let target = path.clone();
                let show = Button::new(&L10N.tr("show-in-files"), move || reveal(target.clone()));
                let controls = Stack::new(
                    Axis::Horizontal,
                    Spacing::M as f32,
                    0.0,
                    vec![boxed(Label::new(&format_size(bytes))), boxed(show)],
                );
                let row = Row::new(Text::new(&name, Tone::Primary), Some(boxed(controls)))
                    .detail(Text::new(&folder, Tone::Muted).caption());
                self.item_row(path.to_string_lossy().into_owned(), row)
            })
            .collect()
    }
}
