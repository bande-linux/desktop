use bande_l10n::{text, tr};

use crate::forget::Forget;
use crate::pages::{Widgets, boxed, muted_row};
use crate::services::NetworkCommand;
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};
use bande_network::VpnState;
use bande_settings_panes::{SettingId, Value};
use bande_toolkit::Spacing;
use bande_toolkit::ui::{Axis, Button, SegmentedControl, Stack, TextField, Widget};

const MANUAL_FIELDS: [(SettingId, bande_l10n::Text, bande_l10n::Text); 3] = [
    (
        SettingId::EthernetAddress,
        text!("IP address"),
        text!("192.168.1.20/24"),
    ),
    (
        SettingId::EthernetRouter,
        text!("Router"),
        text!("192.168.1.1"),
    ),
    (
        SettingId::EthernetDns,
        text!("DNS servers"),
        text!("1.1.1.1, 9.9.9.9"),
    ),
];

impl SettingsView {
    pub(crate) fn manual_field(
        &mut self,
        id: SettingId,
        label: bande_l10n::Text,
        placeholder: bande_l10n::Text,
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
        let field = TextField::new(&placeholder.tr(), &text, move |text| draft(text.to_owned()))
            .without_text_checking();
        self.anchors.push((Anchor::Setting(id, None), field.id()));
        boxed(Row::new(
            Text::new(&label.tr(), Tone::Primary),
            Some(boxed(field)),
        ))
    }

    pub(crate) fn ethernet_rows(&mut self) -> Widgets {
        let Some(network) = self.network.clone() else {
            return Vec::new();
        };
        let Some(ethernet) = network.ethernet else {
            return vec![muted_row(&tr!("No Ethernet port"))];
        };
        let status = if !ethernet.managed {
            tr!("Set up by the system, outside NetworkManager")
        } else if !ethernet.cable {
            tr!("Cable unplugged")
        } else if ethernet.connected {
            match &ethernet.addressing.address {
                Some(address) => {
                    tr!("Connected as {address}", address = address.clone())
                }
                None => tr!("Connected"),
            }
        } else {
            tr!("Not connected")
        };
        let mut detail = Vec::new();
        if let Some(router) = &ethernet.addressing.router {
            detail.push(tr!("Router {address}", address = router.clone()));
        }
        if !ethernet.addressing.dns.is_empty() {
            detail.push(tr!(
                "DNS {servers}",
                servers = ethernet.addressing.dns.join(", ")
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
        let labels = [tr!("Automatically"), tr!("Manually")];
        let choose = intent(&self.intents, |index: usize| Intent::ManualIpv4(index == 1));
        let segmented = SegmentedControl::new(
            &tr!("Configure IPv4"),
            &labels,
            Some(usize::from(self.manual_ipv4)),
            choose,
        );
        self.anchors
            .push((Anchor::Setting(SettingId::Ethernet, None), segmented.id()));
        rows.push(boxed(Row::new(
            Text::new(&tr!("Configure IPv4"), Tone::Primary),
            Some(boxed(segmented)),
        )));
        if self.manual_ipv4 {
            for (id, label, placeholder) in MANUAL_FIELDS {
                rows.push(self.manual_field(id, label, placeholder));
            }
        } else if ethernet.connection.is_some() {
            rows.push(self.manual_field(
                SettingId::EthernetDns,
                text!("DNS servers"),
                text!("Automatic"),
            ));
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
                        tr!("Connected"),
                        tr!("Disconnect"),
                        NetworkCommand::DisconnectVpn,
                    ),
                    VpnState::Connecting => (
                        tr!("Connecting…"),
                        tr!("Disconnect"),
                        NetworkCommand::DisconnectVpn,
                    ),
                    VpnState::Off => (
                        tr!("Not connected"),
                        tr!("Connect"),
                        NetworkCommand::ConnectVpn,
                    ),
                };
            let forget = Forget::Vpn(vpn.path.clone());
            if self.forgetting.as_ref() == Some(&forget) {
                rows.push(self.confirm_row(
                    &tr!("Remove the VPN “{name}”?", name = vpn.name.clone()),
                    &tr!("Its settings and keys are deleted. Import the configuration again to use it."),
                    &tr!("Remove"),
                ));
                continue;
            }
            let mut act = intent(&self.intents, Intent::Network);
            let path = vpn.path.clone();
            let button = Button::new(&action, move || act(command(path.clone())));
            let mut ask = intent(&self.intents, move |()| {
                Intent::AskForget(Some(forget.clone()))
            });
            let remove = Button::new(&tr!("Remove…"), move || ask(()));
            let row = Row::new(
                Text::new(&vpn.name, Tone::Primary),
                Some(boxed(Stack::new(
                    Axis::Horizontal,
                    Spacing::S as f32,
                    0.0,
                    vec![boxed(remove), boxed(button)],
                ))),
            )
            .detail(Text::new(&status, Tone::Muted).caption());
            rows.push(self.item_row(format!("vpn:{}", vpn.path), row));
        }
        let mut import = intent(&self.intents, |()| Intent::ImportVpn);
        let button = Button::new(&tr!("Import Configuration…"), move || import(()));
        self.anchors
            .push((Anchor::Setting(SettingId::Vpns, None), button.id()));
        rows.push(boxed(Row::new(
            Text::new(
                &tr!("Add a WireGuard (.conf) or OpenVPN (.ovpn) file"),
                Tone::Muted,
            )
            .caption(),
            Some(boxed(button)),
        )));
        rows
    }
}
