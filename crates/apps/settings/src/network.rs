use bande_network::VpnState;
use bande_settings_panes::{SettingId, Value};
use bande_toolkit::ui::{Button, SegmentedControl, TextField, Widget};

use crate::L10N;
use crate::pages::{Widgets, boxed, muted_row};
use crate::services::NetworkCommand;
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

const MANUAL_FIELDS: [(SettingId, &str, &str); 3] = [
    (
        SettingId::EthernetAddress,
        "ip-address",
        "ip-address-placeholder",
    ),
    (SettingId::EthernetRouter, "router", "router-placeholder"),
    (SettingId::EthernetDns, "dns-servers", "dns-placeholder"),
];

impl SettingsView {
    pub(crate) fn manual_field(
        &mut self,
        id: SettingId,
        label: &str,
        placeholder: &str,
    ) -> Box<dyn Widget> {
        let text = self
            .drafts
            .get(&id)
            .cloned()
            .unwrap_or_else(|| match self.values.get(&id) {
                Some(Value::Text(text)) => text.clone(),
                _ => String::new(),
            });
        let mut draft = intent(&self.intents, move |text: String| Intent::Draft(id, text));
        let field = TextField::new(&L10N.tr(placeholder), &text, move |text| {
            draft(text.to_owned())
        });
        self.anchors.push((Anchor::Setting(id, None), field.id()));
        boxed(Row::new(
            Text::new(&L10N.tr(label), Tone::Primary),
            Some(boxed(field)),
        ))
    }

    pub(crate) fn ethernet_rows(&mut self) -> Widgets {
        let Some(network) = self.network.clone() else {
            return Vec::new();
        };
        let Some(ethernet) = network.ethernet else {
            return vec![muted_row(&L10N.tr("no-ethernet"))];
        };
        let status = if !ethernet.managed {
            L10N.tr("ethernet-unmanaged")
        } else if !ethernet.cable {
            L10N.tr("cable-unplugged")
        } else if ethernet.connected {
            match &ethernet.addressing.address {
                Some(address) => {
                    L10N.tr_args("connected-as", [("address", address.clone().into())])
                }
                None => L10N.tr("connected"),
            }
        } else {
            L10N.tr("not-connected")
        };
        let mut detail = Vec::new();
        if let Some(router) = &ethernet.addressing.router {
            detail.push(L10N.tr_args("router-is", [("address", router.clone().into())]));
        }
        if !ethernet.addressing.dns.is_empty() {
            detail.push(L10N.tr_args(
                "dns-is",
                [("servers", ethernet.addressing.dns.join(", ").into())],
            ));
        }
        let mut row = Row::new(Text::new(&status, Tone::Primary), None);
        if !detail.is_empty() {
            row = row.detail(Text::new(&detail.join(" · "), Tone::Muted).caption());
        }
        let mut rows = vec![boxed(row)];
        if !ethernet.managed {
            return rows;
        }
        let labels = [L10N.tr("ipv4-automatic"), L10N.tr("ipv4-manual")];
        let choose = intent(&self.intents, |index: usize| Intent::ManualIpv4(index == 1));
        let segmented = SegmentedControl::new(
            &L10N.tr("configure-ipv4"),
            &labels,
            Some(usize::from(self.manual_ipv4)),
            choose,
        );
        self.anchors
            .push((Anchor::Setting(SettingId::Ethernet, None), segmented.id()));
        rows.push(boxed(Row::new(
            Text::new(&L10N.tr("configure-ipv4"), Tone::Primary),
            Some(boxed(segmented)),
        )));
        if self.manual_ipv4 {
            for (id, label, placeholder) in MANUAL_FIELDS {
                rows.push(self.manual_field(id, label, placeholder));
            }
        }
        rows
    }

    pub(crate) fn vpn_rows(&mut self) -> Widgets {
        let Some(network) = self.network.clone() else {
            return Vec::new();
        };
        let mut rows: Widgets = Vec::new();
        for vpn in network.vpns {
            let (status, action, command): (String, String, fn(String) -> NetworkCommand) =
                match vpn.state {
                    VpnState::Connected => (
                        L10N.tr("connected"),
                        L10N.tr("disconnect"),
                        NetworkCommand::DisconnectVpn,
                    ),
                    VpnState::Connecting => (
                        L10N.tr("connecting"),
                        L10N.tr("disconnect"),
                        NetworkCommand::DisconnectVpn,
                    ),
                    VpnState::Off => (
                        L10N.tr("not-connected"),
                        L10N.tr("connect"),
                        NetworkCommand::ConnectVpn,
                    ),
                };
            let mut act = intent(&self.intents, Intent::Network);
            let path = vpn.path.clone();
            let button = Button::new(&action, move || act(command(path.clone())));
            let row = Row::new(Text::new(&vpn.name, Tone::Primary), Some(boxed(button)))
                .detail(Text::new(&status, Tone::Muted).caption());
            rows.push(self.item_row(format!("vpn:{}", vpn.path), row));
        }
        let mut import = intent(&self.intents, |()| Intent::ImportVpn);
        let button = Button::new(&L10N.tr("import-vpn"), move || import(()));
        self.anchors
            .push((Anchor::Setting(SettingId::Vpns, None), button.id()));
        rows.push(boxed(Row::new(
            Text::new(&L10N.tr("import-vpn-hint"), Tone::Muted).caption(),
            Some(boxed(button)),
        )));
        rows
    }
}
