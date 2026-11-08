use std::path::Path;

use bande_l10n::tr;

use crate::pages::{Widgets, boxed, muted_row};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};
use bande_settings_panes::SettingId;
use bande_toolkit::Spacing;
use bande_toolkit::ui::{Axis, Button, Stack, Widget};
use zbus::blocking::Connection;

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
            let erase = Button::new(&tr!("Clear"), move || clear(()));
            self.anchors
                .push((Anchor::Item(CONFIRM.to_owned()), erase.id()));
            let buttons = Stack::new(
                Axis::Horizontal,
                Spacing::S as f32,
                0.0,
                vec![
                    boxed(Button::new(&tr!("Cancel"), move || cancel(())).cancel()),
                    boxed(erase.primary()),
                ],
            );
            return vec![boxed(
                Row::new(
                    Text::new(&tr!("Clear the clipboard history?"), Tone::Primary),
                    Some(boxed(buttons)),
                )
                .detail(
                    Text::new(
                        &tr!("Everything you copied earlier is forgotten."),
                        Tone::Muted,
                    )
                    .caption(),
                )
                .highlighted(true),
            )];
        }
        let mut ask = intent(&self.intents, |()| Intent::AskClearClipboard(true));
        let button = Button::new(&tr!("Clear History…"), move || ask(()));
        self.anchors
            .push((Anchor::Item(ASK.to_owned()), button.id()));
        vec![boxed(
            Row::new(
                Text::new(&tr!("Clipboard history"), Tone::Primary),
                Some(boxed(button)),
            )
            .detail(
                Text::new(
                    &tr!("What you copied earlier, ready to paste again"),
                    Tone::Muted,
                )
                .caption(),
            ),
        )]
    }
}

impl SettingsView {
    pub(crate) fn excluded_rows(&mut self) -> Widgets {
        let excluded = self.excluded_folders();
        let mut rows: Widgets = Vec::new();
        if excluded.is_empty() {
            rows.push(muted_row(&tr!(
                "Every folder in your home folder is searched"
            )));
        }
        for folder in excluded {
            let name = Path::new(&folder).file_name().map_or_else(
                || folder.clone(),
                |name| name.to_string_lossy().into_owned(),
            );
            let mut include = intent(&self.intents, Intent::IncludeFolder);
            let path = folder.clone();
            let button = Button::new(&tr!("Remove"), move || include(path.clone()))
                .tooltip(&tr!("Search this folder again"));
            let row = Row::new(Text::new(&name, Tone::Primary), Some(boxed(button)))
                .detail(Text::new(&folder, Tone::Muted).caption());
            rows.push(self.item_row(format!("excluded:{folder}"), row));
        }
        let mut add = intent(&self.intents, |()| Intent::ExcludeFolder);
        let button = Button::new(&tr!("Add Folder…"), move || add(()));
        self.anchors.push((
            Anchor::Setting(SettingId::SearchExcluded, None),
            button.id(),
        ));
        rows.push(boxed(Row::new(
            Text::new(
                &tr!("Files in these folders never appear in search"),
                Tone::Muted,
            )
            .caption(),
            Some(boxed(button)),
        )));
        rows
    }
}

pub(crate) fn focus_after_asking(asking: bool) -> Anchor {
    Anchor::Item(if asking { CONFIRM } else { ASK }.to_owned())
}
