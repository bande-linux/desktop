use std::path::Path;

use bande_toolkit::Spacing;
use bande_toolkit::ui::{Axis, Button, Stack, Widget};
use zbus::blocking::Connection;

use crate::L10N;
use crate::pages::{Widgets, boxed, muted_row};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};
use bande_settings_panes::SettingId;

const CLIPBOARD: &str = "org.bande.Clipboard";
const CLIPBOARD_PATH: &str = "/org/bande/Clipboard";
const ASK: &str = "clear-clipboard";
const CONFIRM: &str = "clear-clipboard-confirm";

pub fn clear_clipboard(session: &Connection) -> zbus::Result<()> {
    zbus::blocking::Proxy::new(session, CLIPBOARD, CLIPBOARD_PATH, CLIPBOARD)?.call("Clear", &())
}

impl SettingsView {
    pub(crate) fn clipboard_rows(&mut self) -> Widgets {
        if self.confirming_clear_clipboard {
            let mut cancel = intent(&self.intents, |()| Intent::AskClearClipboard(false));
            let mut clear = intent(&self.intents, |()| Intent::ClearClipboard);
            let erase = Button::new(&L10N.tr("clear"), move || clear(()));
            self.anchors
                .push((Anchor::Item(CONFIRM.to_owned()), erase.id()));
            let buttons = Stack::new(
                Axis::Horizontal,
                Spacing::S as f32,
                0.0,
                vec![
                    boxed(Button::new(&L10N.tr("cancel"), move || cancel(())).cancel()),
                    boxed(erase.primary()),
                ],
            );
            return vec![boxed(
                Row::new(
                    Text::new(&L10N.tr("clear-clipboard-question"), Tone::Primary),
                    Some(boxed(buttons)),
                )
                .detail(Text::new(&L10N.tr("clear-clipboard-warning"), Tone::Muted).caption())
                .highlighted(true),
            )];
        }
        let mut ask = intent(&self.intents, |()| Intent::AskClearClipboard(true));
        let button = Button::new(&L10N.tr("clear-clipboard-ellipsis"), move || ask(()));
        self.anchors
            .push((Anchor::Item(ASK.to_owned()), button.id()));
        vec![boxed(
            Row::new(
                Text::new(&L10N.tr("clipboard-history"), Tone::Primary),
                Some(boxed(button)),
            )
            .detail(Text::new(&L10N.tr("clipboard-history-hint"), Tone::Muted).caption()),
        )]
    }
}

impl SettingsView {
    pub(crate) fn excluded_rows(&mut self) -> Widgets {
        let excluded = self.excluded_folders();
        let mut rows: Widgets = Vec::new();
        if excluded.is_empty() {
            rows.push(muted_row(&L10N.tr("no-excluded-folders")));
        }
        for folder in excluded {
            let name = Path::new(&folder).file_name().map_or_else(
                || folder.clone(),
                |name| name.to_string_lossy().into_owned(),
            );
            let mut include = intent(&self.intents, Intent::IncludeFolder);
            let path = folder.clone();
            let button = Button::new(&L10N.tr("remove"), move || include(path.clone()))
                .tooltip(&L10N.tr("include-folder-tooltip"));
            let row = Row::new(Text::new(&name, Tone::Primary), Some(boxed(button)))
                .detail(Text::new(&folder, Tone::Muted).caption());
            rows.push(self.item_row(format!("excluded:{folder}"), row));
        }
        let mut add = intent(&self.intents, |()| Intent::ExcludeFolder);
        let button = Button::new(&L10N.tr("exclude-folder"), move || add(()));
        self.anchors.push((
            Anchor::Setting(SettingId::SearchExcluded, None),
            button.id(),
        ));
        rows.push(boxed(Row::new(
            Text::new(&L10N.tr("exclude-folder-hint"), Tone::Muted).caption(),
            Some(boxed(button)),
        )));
        rows
    }
}

pub(crate) fn focus_after_asking(asking: bool) -> Anchor {
    Anchor::Item(if asking { CONFIRM } else { ASK }.to_owned())
}
