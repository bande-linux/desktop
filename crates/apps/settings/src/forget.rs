use unbound_toolkit::Spacing;
use unbound_toolkit::ui::{Axis, Button, Stack, Widget};

use crate::L10N;
use crate::follow::Command;
use crate::pages::boxed;
use crate::services::{BluetoothCommand, NetworkCommand};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

const CONFIRM: &str = "forget-confirm";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Forget {
    Network(Vec<u8>),
    Device(String),
}

impl SettingsView {
    pub(crate) fn forget_button(&self, target: Forget) -> Box<dyn Widget> {
        let mut ask = intent(&self.intents, move |()| {
            Intent::AskForget(Some(target.clone()))
        });
        boxed(Button::new(&L10N.tr("forget-ellipsis"), move || ask(())))
    }

    pub(crate) fn forget_row(&mut self, name: &str, warning: &str) -> Box<dyn Widget> {
        let mut cancel = intent(&self.intents, |()| Intent::AskForget(None));
        let mut forget = intent(&self.intents, |()| Intent::Forget);
        let confirm = Button::new(&L10N.tr("forget"), move || forget(()));
        self.anchors
            .push((Anchor::Item(CONFIRM.to_owned()), confirm.id()));
        let buttons = Stack::new(
            Axis::Horizontal,
            Spacing::S as f32,
            0.0,
            vec![
                boxed(Button::new(&L10N.tr("cancel"), move || cancel(())).cancel()),
                boxed(confirm.primary()),
            ],
        );
        boxed(
            Row::new(
                Text::new(
                    &L10N.tr_args("forget-question", [("name", name.to_owned().into())]),
                    Tone::Primary,
                ),
                Some(boxed(buttons)),
            )
            .detail(Text::new(warning, Tone::Muted).caption())
            .highlighted(true),
        )
    }

    pub(crate) fn ask_forget(&mut self, target: Option<Forget>) {
        let back = match target.as_ref().or(self.forgetting.as_ref()) {
            Some(Forget::Network(ssid)) => String::from_utf8_lossy(ssid).into_owned(),
            Some(Forget::Device(address)) => address.clone(),
            None => return,
        };
        self.focus_next = Some(Anchor::Item(if target.is_some() {
            CONFIRM.to_owned()
        } else {
            back
        }));
        self.forgetting = target;
    }

    pub(crate) fn forget(&mut self) {
        let command = match self.forgetting.take() {
            Some(Forget::Network(ssid)) => self
                .wifi
                .as_ref()
                .and_then(|wifi| wifi.networks.iter().find(|network| network.ssid == ssid))
                .cloned()
                .map(|network| Command::Network(NetworkCommand::Forget(network))),
            Some(Forget::Device(address)) => self
                .bluetooth
                .as_ref()
                .and_then(|bluetooth| {
                    bluetooth
                        .paired
                        .iter()
                        .find(|device| device.address == address)
                })
                .cloned()
                .map(|device| Command::Bluetooth(BluetoothCommand::Forget(device))),
            None => None,
        };
        self.commands.extend(command);
    }
}
