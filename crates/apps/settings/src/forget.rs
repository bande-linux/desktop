use bande_settings_panes::{Permission, SettingId};
use bande_toolkit::Spacing;
use bande_toolkit::ui::{Axis, Button, Stack, Widget};

use crate::L10N;
use crate::follow::Command;
use crate::pages::boxed;
use crate::permissions::PermissionCommand;
use crate::services::{BluetoothCommand, NetworkCommand};
use crate::view::{Anchor, Detail, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

const CONFIRM: &str = "forget-confirm";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Forget {
    Network(Vec<u8>),
    Device(String),
    Share(String),
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
            Some(Forget::Share(app)) => format!("grant:{app}"),
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
            Some(Forget::Network(ssid)) => {
                if self.detail == Some(Detail::Network(ssid.clone())) {
                    self.detail = None;
                    self.focus_next = Some(Anchor::Setting(SettingId::KnownNetworks, None));
                }
                Some(Command::Network(NetworkCommand::Forget(ssid)))
            }
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
            Some(Forget::Share(app)) => {
                if let Some((_, grants)) = self
                    .permissions
                    .iter_mut()
                    .find(|(permission, _)| *permission == Permission::ScreenSharing)
                {
                    grants.retain(|grant| grant.app != app);
                }
                Some(Command::Permission(PermissionCommand::Forget(
                    Permission::ScreenSharing,
                    app,
                )))
            }
            None => None,
        };
        self.commands.extend(command);
    }
}
