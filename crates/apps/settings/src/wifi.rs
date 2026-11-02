use bande_network::{ConnectionChange, KnownNetwork, Network, PrivateAddress, Security};
use bande_settings_panes::{PaneId, SettingId, Value};
use bande_toolkit::ui::{
    Axis, Button, SecureField, SegmentedControl, Stack, TextField, Toggle, Widget,
};
use bande_toolkit::{Spacing, TextRole};

use crate::L10N;
use crate::forget::Forget;
use crate::general::info_row;
use crate::pages::{FIELD_SIZE, Widgets, boxed, muted_row, submit};
use crate::services::NetworkCommand;
use crate::view::{Anchor, Detail, Intent, SettingsView, intent};
use crate::widgets::{Caption, Card, HEADER_SYMBOL, Header, Row, Signal, Text, Tone};

pub(crate) const WIFI_FIELDS: [(SettingId, &str, &str); 3] = [
    (
        SettingId::WifiAddress,
        "ip-address",
        "ip-address-placeholder",
    ),
    (SettingId::WifiRouter, "router", "router-placeholder"),
    (SettingId::WifiDns, "dns-servers", "dns-placeholder"),
];

pub(crate) const HIDDEN_SECURITY: [(Security, &str); 3] = [
    (Security::Open, "security-none"),
    (Security::Personal { sae_only: false }, "security-wpa2"),
    (Security::Personal { sae_only: true }, "security-wpa3"),
];

const PRIVATE_ADDRESS: [(PrivateAddress, &str); 3] = [
    (PrivateAddress::Off, "private-address-off"),
    (PrivateAddress::Fixed, "private-address-fixed"),
    (PrivateAddress::Rotating, "private-address-rotating"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Other {
    pub name: String,
    pub security: Security,
}

impl Default for Other {
    fn default() -> Self {
        Self {
            name: String::new(),
            security: Security::Personal { sae_only: false },
        }
    }
}

fn ssid_text(ssid: &[u8]) -> String {
    String::from_utf8_lossy(ssid).into_owned()
}

impl SettingsView {
    fn nearby(&self, ssid: &[u8]) -> Option<&Network> {
        self.wifi
            .as_ref()?
            .networks
            .iter()
            .find(|network| network.ssid == ssid)
    }

    fn known_status(&self, known: &KnownNetwork) -> String {
        let nearby = self.nearby(&known.ssid);
        let current = nearby.is_some_and(|network| network.current);
        if current && self.wifi.as_ref().is_some_and(|wifi| wifi.sign_in) {
            L10N.tr("sign-in-required")
        } else if current {
            L10N.tr("connected")
        } else if self.join.joining.as_ref() == Some(&known.ssid) {
            L10N.tr("joining")
        } else if nearby.is_none() {
            L10N.tr("not-in-range")
        } else if !known.auto_join {
            L10N.tr("auto-join-off")
        } else {
            L10N.tr("saved")
        }
    }

    fn details_button(&self, ssid: &[u8]) -> Box<dyn Widget> {
        let mut open = intent(&self.intents, Intent::OpenNetwork);
        let ssid = ssid.to_vec();
        boxed(Button::new(&L10N.tr("details-ellipsis"), move || {
            open(ssid.clone())
        }))
    }

    pub(crate) fn known_network_rows(&mut self) -> Widgets {
        let Some(wifi) = self.wifi.clone() else {
            return Vec::new();
        };
        if wifi.known.is_empty() {
            return vec![muted_row(&L10N.tr("no-known-networks"))];
        }
        let mut known = wifi.known.clone();
        known.sort_by_key(|network| {
            let nearby = self.nearby(&network.ssid);
            (
                !nearby.is_some_and(|nearby| nearby.current),
                nearby.is_none(),
                std::cmp::Reverse(nearby.map_or(0, |nearby| nearby.strength)),
            )
        });
        let mut rows = Vec::new();
        for network in known {
            let name = network.name();
            let nearby = self.nearby(&network.ssid).cloned();
            let mut controls = Vec::new();
            if wifi.sign_in && nearby.as_ref().is_some_and(|nearby| nearby.current) {
                let mut sign_in =
                    intent(&self.intents, |()| Intent::Network(NetworkCommand::SignIn));
                controls.push(boxed(
                    Button::new(&L10N.tr("sign-in-ellipsis"), move || sign_in(())).primary(),
                ));
            }
            controls.push(self.details_button(&network.ssid));
            if let Some(nearby) = &nearby {
                controls.push(boxed(Signal::new(
                    nearby.strength,
                    nearby.security.needs_password(),
                )));
            }
            let mut row = Row::new(
                Text::new(&name, Tone::Primary),
                Some(boxed(Stack::new(
                    Axis::Horizontal,
                    Spacing::S as f32,
                    0.0,
                    controls,
                ))),
            )
            .detail(Text::new(&self.known_status(&network), Tone::Muted).caption());
            if nearby.is_some_and(|nearby| !nearby.current) {
                let mut join = intent(&self.intents, Intent::Join);
                let ssid = network.ssid.clone();
                row = row.on_activate(move || join(ssid.clone()));
            }
            rows.push(self.item_row(name, row));
        }
        rows
    }

    fn password_row(&mut self, name: &str) -> Box<dyn Widget> {
        let field = SecureField::new(
            &L10N.tr("password"),
            FIELD_SIZE,
            submit(&self.intents, Intent::Password),
        );
        self.anchors.push((Anchor::Password, field.id()));
        let detail = self
            .join
            .error
            .clone()
            .unwrap_or_else(|| L10N.tr("password-hint"));
        boxed(
            Row::new(
                Text::new(
                    &L10N.tr_args("password-for", [("network", name.to_owned().into())]),
                    Tone::Primary,
                ),
                Some(boxed(field)),
            )
            .detail(Text::new(&detail, Tone::Muted).caption()),
        )
    }

    pub(crate) fn network_rows(&mut self) -> Widgets {
        let Some(wifi) = self.wifi.clone() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        let scanned = !wifi.networks.is_empty();
        let others: Vec<Network> = wifi
            .networks
            .into_iter()
            .filter(|network| !network.known)
            .collect();
        if !scanned {
            rows.push(muted_row(&L10N.tr("searching-networks")));
        }
        for network in others {
            let name = network.name();
            let mut row = Row::new(
                Text::new(&name, Tone::Primary),
                Some(boxed(Signal::new(
                    network.strength,
                    network.security.needs_password(),
                ))),
            );
            if self.join.joining.as_ref() == Some(&network.ssid) {
                row = row.detail(Text::new(&L10N.tr("joining"), Tone::Muted).caption());
            }
            let mut join = intent(&self.intents, Intent::Join);
            let ssid = network.ssid.clone();
            row = row.on_activate(move || join(ssid.clone()));
            rows.push(self.item_row(name.clone(), row));
            if self.join.asking.as_ref() == Some(&network.ssid) {
                rows.push(self.password_row(&name));
            }
        }
        rows.extend(self.other_rows());
        rows
    }

    fn other_rows(&mut self) -> Widgets {
        let Some(other) = self.other.clone() else {
            let mut ask = intent(&self.intents, |()| Intent::AskOther(true));
            let button = Button::new(&L10N.tr("other-network-ellipsis"), move || ask(()));
            self.anchors
                .push((Anchor::Item("other-network".to_owned()), button.id()));
            return vec![boxed(Row::new(
                Text::new(&L10N.tr("other-network-hint"), Tone::Muted).caption(),
                Some(boxed(button)),
            ))];
        };
        let mut rename = intent(&self.intents, Intent::OtherName);
        let field = TextField::new(&L10N.tr("network-name"), &other.name, move |text| {
            rename(text.to_owned())
        });
        self.anchors
            .push((Anchor::Setting(SettingId::OtherNetwork, None), field.id()));
        let mut rows = vec![boxed(Row::new(
            Text::new(&L10N.tr("network-name"), Tone::Primary),
            Some(boxed(field)),
        ))];
        let labels: Vec<String> = HIDDEN_SECURITY
            .iter()
            .map(|(_, label)| L10N.tr(label))
            .collect();
        let selected = HIDDEN_SECURITY
            .iter()
            .position(|(security, _)| *security == other.security);
        let choose = intent(&self.intents, Intent::OtherSecurity);
        let segmented = SegmentedControl::new(&L10N.tr("security"), &labels, selected, choose);
        self.anchors
            .push((Anchor::Item("other-security".to_owned()), segmented.id()));
        rows.push(boxed(Row::new(
            Text::new(&L10N.tr("security"), Tone::Primary),
            Some(boxed(segmented)),
        )));
        let joining = self.join.joining.as_deref() == Some(other.name.as_bytes());
        let detail = if joining {
            L10N.tr("joining")
        } else {
            self.join
                .error
                .clone()
                .unwrap_or_else(|| L10N.tr("other-network-detail"))
        };
        let mut cancel = intent(&self.intents, |()| Intent::AskOther(false));
        let mut buttons = vec![boxed(
            Button::new(&L10N.tr("cancel"), move || cancel(())).cancel(),
        )];
        if other.security.needs_password() {
            let field = SecureField::new(
                &L10N.tr("password"),
                FIELD_SIZE,
                submit(&self.intents, |password| Intent::JoinOther(Some(password))),
            );
            self.anchors.push((Anchor::Password, field.id()));
            rows.push(boxed(Row::new(
                Text::new(&L10N.tr("password"), Tone::Primary),
                Some(boxed(field)),
            )));
        } else {
            let mut join = intent(&self.intents, |()| Intent::JoinOther(None));
            buttons.push(boxed(
                Button::new(&L10N.tr("join"), move || join(())).primary(),
            ));
        }
        rows.push(boxed(Row::new(
            Text::new(&detail, Tone::Muted).caption(),
            Some(boxed(Stack::new(
                Axis::Horizontal,
                Spacing::S as f32,
                0.0,
                buttons,
            ))),
        )));
        rows
    }

    fn wifi_switch(
        &mut self,
        label: &str,
        on: bool,
        path: &str,
        change: fn(bool) -> ConnectionChange,
    ) -> Box<dyn Widget> {
        let path = path.to_owned();
        let set = intent(&self.intents, move |on: bool| {
            Intent::Network(NetworkCommand::ChangeConnection(path.clone(), change(on)))
        });
        let toggle = Toggle::new(&L10N.tr(label), on, set);
        self.anchors
            .push((Anchor::Item(label.to_owned()), toggle.id()));
        boxed(
            Row::new(
                Text::new(&L10N.tr(label), Tone::Primary),
                Some(boxed(toggle)),
            )
            .detail(Text::new(&L10N.tr(&format!("{label}-detail")), Tone::Muted).caption()),
        )
    }

    pub(crate) fn network_page(&mut self) -> Option<Widgets> {
        let Some(Detail::Network(ssid)) = self.detail.clone() else {
            return None;
        };
        let wifi = self.wifi.clone()?;
        let known = wifi.known.iter().find(|known| known.ssid == ssid)?.clone();
        let current = self.nearby(&ssid).is_some_and(|network| network.current);
        let name = known.name();
        let mut page: Widgets = self.back_button(PaneId::Wifi).into_iter().collect();
        let ink = self.palette().ink;
        let symbol = self.symbol(PaneId::Wifi.symbol(), HEADER_SYMBOL, ink);
        let status = self.known_status(&known);
        page.push(boxed(Header::new(
            symbol,
            Text::new(&name, Tone::Primary).role(TextRole::Title),
            &status,
            None,
        )));
        let mut choices = vec![
            self.wifi_switch(
                "auto-join",
                known.auto_join,
                &known.path,
                ConnectionChange::AutoJoin,
            ),
            self.wifi_switch(
                "low-data-mode",
                known.low_data,
                &known.path,
                ConnectionChange::LowData,
            ),
        ];
        let labels: Vec<String> = PRIVATE_ADDRESS
            .iter()
            .map(|(_, label)| L10N.tr(label))
            .collect();
        let selected = PRIVATE_ADDRESS
            .iter()
            .position(|(choice, _)| *choice == known.private_address);
        let path = known.path.clone();
        let choose = intent(&self.intents, move |index: usize| {
            let choice = PRIVATE_ADDRESS
                .get(index)
                .map_or(PrivateAddress::Off, |(choice, _)| *choice);
            Intent::Network(NetworkCommand::ChangeConnection(
                path.clone(),
                ConnectionChange::PrivateAddress(choice),
            ))
        });
        let segmented =
            SegmentedControl::new(&L10N.tr("private-address"), &labels, selected, choose);
        self.anchors
            .push((Anchor::Item("private-address".to_owned()), segmented.id()));
        choices.push(boxed(
            Row::new(
                Text::new(&L10N.tr("private-address"), Tone::Primary),
                Some(boxed(segmented)),
            )
            .detail(Text::new(&L10N.tr("private-address-detail"), Tone::Muted).caption()),
        ));
        page.push(boxed(Card::new(choices)));
        if current {
            let link = &wifi.link;
            let mut rows = Vec::new();
            if let Some(address) = &link.addressing.address {
                rows.push(info_row(&L10N.tr("ip-address"), address));
            }
            if let Some(router) = &link.addressing.router {
                rows.push(info_row(&L10N.tr("router"), router));
            }
            if !link.addressing.dns.is_empty() {
                rows.push(info_row(
                    &L10N.tr("dns-servers"),
                    &link.addressing.dns.join(", "),
                ));
            }
            if !link.hardware.is_empty() {
                rows.push(info_row(&L10N.tr("wifi-address"), &link.hardware));
            }
            if !rows.is_empty() {
                page.push(boxed(Caption::new(Text::label(&L10N.tr("connection")))));
                page.push(boxed(Card::new(rows)));
            }
        }
        let labels = [L10N.tr("ipv4-automatic"), L10N.tr("ipv4-manual")];
        let mut choose = intent(&self.intents, Intent::WifiManual);
        let segmented = SegmentedControl::new(
            &L10N.tr("configure-ipv4"),
            &labels,
            Some(usize::from(self.wifi_manual)),
            move |index| choose(index == 1),
        );
        self.anchors
            .push((Anchor::Item("wifi-ipv4".to_owned()), segmented.id()));
        let mut addressing = vec![boxed(Row::new(
            Text::new(&L10N.tr("configure-ipv4"), Tone::Primary),
            Some(boxed(segmented)),
        ))];
        if self.wifi_manual {
            for (id, label, placeholder) in WIFI_FIELDS {
                addressing.push(self.manual_field(id, label, placeholder));
            }
        } else {
            addressing.push(self.manual_field(
                SettingId::WifiDns,
                "dns-servers",
                "dns-automatic-placeholder",
            ));
        }
        page.push(boxed(Caption::new(Text::label(&L10N.tr("tcp-ip")))));
        page.push(boxed(Card::new(addressing)));
        if let Some(message) = &self.message {
            page.push(boxed(Caption::new(Text::new(message, Tone::Muted))));
        }
        let forget = if self.forgetting.as_ref() == Some(&Forget::Network(ssid.clone())) {
            self.forget_row(&name, &L10N.tr("forget-network-warning"))
        } else {
            let button = self.forget_button(Forget::Network(ssid.clone()));
            self.anchors
                .push((Anchor::Item(ssid_text(&ssid)), button.id()));
            boxed(Row::new(
                Text::new(&L10N.tr("forget-network"), Tone::Primary),
                Some(button),
            ))
        };
        page.push(boxed(Card::new(vec![forget])));
        Some(page)
    }

    pub(crate) fn open_network(&mut self, ssid: Vec<u8>) {
        let known = self
            .wifi
            .as_ref()
            .and_then(|wifi| wifi.known.iter().find(|known| known.ssid == ssid))
            .cloned();
        let Some(known) = known else {
            return;
        };
        let link = self
            .wifi
            .as_ref()
            .filter(|_| self.nearby(&ssid).is_some_and(|network| network.current))
            .map(|wifi| wifi.link.addressing.clone())
            .unwrap_or_default();
        let values = match &known.manual {
            Some(manual) => [
                format!("{}/{}", manual.address, manual.prefix),
                manual
                    .router
                    .map(|router| router.to_string())
                    .unwrap_or_default(),
                manual
                    .dns
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
            ],
            None => [
                link.address.unwrap_or_default(),
                link.router.unwrap_or_default(),
                known
                    .dns
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
            ],
        };
        for ((id, _, _), value) in WIFI_FIELDS.iter().zip(values) {
            self.values.insert(*id, Value::Text(value));
        }
        self.wifi_manual = known.manual.is_some();
        self.detail = Some(Detail::Network(ssid));
        self.forgetting = None;
        self.message = None;
        self.focus_next = Some(Anchor::Back);
    }

    pub(crate) fn change_wifi_addressing(&mut self, id: SettingId) {
        let Some(Detail::Network(ssid)) = &self.detail else {
            return;
        };
        let Some(path) = self.wifi.as_ref().and_then(|wifi| {
            wifi.known
                .iter()
                .find(|known| known.ssid == *ssid)
                .map(|known| known.path.clone())
        }) else {
            return;
        };
        let text = |id| match self.values.get(&id) {
            Some(Value::Text(text)) => text.clone(),
            _ => String::new(),
        };
        let change = if self.wifi_manual {
            bande_network::parse_manual(
                &text(SettingId::WifiAddress),
                &text(SettingId::WifiRouter),
                &text(SettingId::WifiDns),
            )
            .map(|manual| ConnectionChange::Ipv4(Some(manual)))
        } else if id == SettingId::WifiDns {
            bande_network::parse_dns(&text(SettingId::WifiDns)).map(ConnectionChange::Dns)
        } else {
            return;
        };
        self.message = match change {
            Some(change) => {
                self.commands.push(crate::follow::Command::Network(
                    NetworkCommand::ChangeConnection(path, change),
                ));
                None
            }
            None => Some(L10N.tr(if self.wifi_manual {
                "manual-address-invalid"
            } else {
                "dns-invalid"
            })),
        };
    }

    pub(crate) fn set_wifi_manual(&mut self, manual: bool) {
        self.wifi_manual = manual;
        let dns = match self.wifi.as_ref() {
            Some(wifi) if manual => wifi.link.addressing.dns.join(", "),
            _ => String::new(),
        };
        if !manual || self.values.get(&SettingId::WifiDns) == Some(&Value::Text(String::new())) {
            self.values.insert(SettingId::WifiDns, Value::Text(dns));
        }
        if manual {
            return;
        }
        let Some(Detail::Network(ssid)) = &self.detail else {
            return;
        };
        if let Some(known) = self
            .wifi
            .as_ref()
            .and_then(|wifi| wifi.known.iter().find(|known| known.ssid == *ssid))
            .filter(|known| known.manual.is_some())
        {
            self.commands.push(crate::follow::Command::Network(
                NetworkCommand::ChangeConnection(known.path.clone(), ConnectionChange::Ipv4(None)),
            ));
        }
    }

    pub(crate) fn join_other(&mut self, password: Option<String>) {
        let Some(other) = self.other.clone() else {
            return;
        };
        let name = other.name.trim().to_owned();
        if name.is_empty() {
            self.join.error = Some(L10N.tr("network-name-needed"));
            self.focus_next = Some(Anchor::Setting(SettingId::OtherNetwork, None));
            return;
        }
        let ssid = name.into_bytes();
        self.join = crate::view::Join {
            joining: Some(ssid.clone()),
            ..crate::view::Join::default()
        };
        self.commands
            .push(crate::follow::Command::Network(NetworkCommand::JoinHidden(
                ssid,
                other.security,
                password,
            )));
    }
}
