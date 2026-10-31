use unbound_toolkit::Spacing;
use unbound_toolkit::ui::{Axis, Button, Stack, Widget};
use zbus::blocking::Connection;

use crate::L10N;
use crate::pages::{Widgets, boxed};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

const CLIPBOARD: &str = "org.unbound.Clipboard";
const CLIPBOARD_PATH: &str = "/org/unbound/Clipboard";
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

pub(crate) fn focus_after_asking(asking: bool) -> Anchor {
    Anchor::Item(if asking { CONFIRM } else { ASK }.to_owned())
}
