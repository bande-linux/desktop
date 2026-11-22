use bande_l10n::{text, tr};
use bande_network::{Addressing, ConnectionChange, Ethernet, Ipv6, Outside, VpnState};
use bande_settings_panes::{PaneId, SettingId, Value};
use bande_toolkit::ui::{Axis, Button, SegmentedControl, Stack, TextField, Widget};
use bande_toolkit::{Spacing, TextRole};

use crate::follow::Command;
use crate::forget::Forget;
use crate::handover::{HandOver, HandOverCommand};
use crate::pages::{Widgets, boxed, muted_row};
use crate::proxies::{ProxyMode, ProxySettings};
use crate::services::NetworkCommand;
use crate::view::{Anchor, Detail, Intent, SettingsView, intent};
use crate::widgets::{Caption, Card, HEADER_SYMBOL, Header, Row, Text, Tone};

type Fields<const N: usize> = [(SettingId, bande_l10n::Text, bande_l10n::Text); N];

const MANUAL_FIELDS: Fields<3> = [
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
        text!("192.0.2.1, 198.51.100.1"),
    ),
];

pub(crate) const ETHERNET_IPV6_FIELDS: Fields<2> = [
    (
        SettingId::EthernetIpv6Address,
        text!("IPv6 address"),
        text!("2001:db8::20/64"),
    ),
    (
        SettingId::EthernetIpv6Router,
        text!("IPv6 router"),
        text!("2001:db8::1"),
    ),
];

const PROXY_FIELDS: Fields<3> = [
    (
        SettingId::ProxyHttp,
        text!("Web proxy"),
        text!("proxy.example.org:3128"),
    ),
    (
        SettingId::ProxyHttps,
        text!("Secure web proxy"),
        text!("Same as the web proxy"),
    ),
    (
        SettingId::ProxyIgnore,
        text!("Reach directly"),
        text!("localhost, .example.org"),
    ),
];

const IPV6_AUTOMATIC: usize = 0;
const IPV6_MANUAL: usize = 1;
const IPV6_OFF: usize = 2;

fn status_of(connected: bool, addressing: &Addressing) -> String {
    match (&addressing.address, connected) {
        (Some(address), _) => tr!("Connected as {address}", address = address.clone()),
        (None, true) => tr!("Connected"),
        (None, false) => tr!("Not connected"),
    }
}

fn addressing_detail(addressing: &Addressing) -> Option<Text> {
    let mut detail = Vec::new();
    if let Some(router) = &addressing.router {
        detail.push(tr!("Router {address}", address = router.clone()));
    }
    if !addressing.dns.is_empty() {
        detail.push(tr!("DNS {servers}", servers = addressing.dns.join(", ")));
    }
    (!detail.is_empty()).then(|| Text::new(&detail.join(" · "), Tone::Muted).caption())
}

fn wired_status(ethernet: &Ethernet) -> String {
    if ethernet.outside.is_none() && !ethernet.cable {
        tr!("Cable unplugged")
    } else {
        status_of(ethernet.connected, &ethernet.addressing)
    }
}

fn text_of(view: &SettingsView, id: SettingId) -> String {
    match view.values.get(&id) {
        Some(Value::Text(text)) => text.clone(),
        _ => String::new(),
    }
}

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
            .unwrap_or_else(|| text_of(self, id));
        let mut draft = intent(&self.intents, move |text: String| Intent::Draft(id, text));
        let field = TextField::new(&placeholder.tr(), &text, move |text| draft(text.to_owned()))
            .without_text_checking();
        self.anchors.push((Anchor::Setting(id, None), field.id()));
        boxed(Row::new(
            Text::new(&label.tr(), Tone::Primary),
            Some(boxed(field)),
        ))
    }

    pub(crate) fn ipv6_rows(&mut self, pane: PaneId, mode: usize, fields: &Fields<2>) -> Widgets {
        let labels = [tr!("Automatically"), tr!("Manually"), tr!("Off")];
        let segmented = SegmentedControl::new(
            &tr!("Configure IPv6"),
            &labels,
            Some(mode),
            intent(&self.intents, move |index| Intent::Ipv6Mode(pane, index)),
        );
        self.anchors
            .push((Anchor::Item(format!("ipv6:{pane:?}")), segmented.id()));
        let mut rows = vec![boxed(Row::new(
            Text::new(&tr!("Configure IPv6"), Tone::Primary),
            Some(boxed(segmented)),
        ))];
        if mode == IPV6_MANUAL {
            for (id, label, placeholder) in *fields {
                rows.push(self.manual_field(id, label, placeholder));
            }
        }
        rows
    }

    pub(crate) fn load_ipv6(&mut self, ipv6: Ipv6, fields: &Fields<2>) -> usize {
        let (address, router) = match ipv6 {
            Ipv6::Manual(manual) => (
                format!("{}/{}", manual.address, manual.prefix),
                manual
                    .router
                    .map(|router| router.to_string())
                    .unwrap_or_default(),
            ),
            Ipv6::Automatic | Ipv6::Off => (String::new(), String::new()),
        };
        self.values.insert(fields[0].0, Value::Text(address));
        self.values.insert(fields[1].0, Value::Text(router));
        match ipv6 {
            Ipv6::Automatic => IPV6_AUTOMATIC,
            Ipv6::Manual(_) => IPV6_MANUAL,
            Ipv6::Off => IPV6_OFF,
        }
    }

    pub(crate) fn change_ipv6(&mut self, pane: PaneId, path: String, fields: &Fields<2>) {
        let parsed = bande_network::parse_manual_ipv6(
            &text_of(self, fields[0].0),
            &text_of(self, fields[1].0),
        );
        match parsed {
            Some(manual) => {
                self.clear_failure();
                self.commands
                    .push(Command::Network(NetworkCommand::ChangeConnection(
                        pane,
                        path,
                        ConnectionChange::Ipv6(Ipv6::Manual(manual)),
                    )));
            }
            None => self.fail_here(tr!(
                "Type an address such as 2001:db8::20/64, and the router as an address such as 2001:db8::1."
            )),
        }
    }

    fn connection_path(&self, pane: PaneId) -> Option<String> {
        match (pane, self.detail.as_ref()?) {
            (PaneId::Wifi, Detail::Network(ssid)) => self
                .wifi
                .present()?
                .known
                .iter()
                .find(|known| known.ssid == *ssid)
                .map(|known| known.path.clone()),
            (PaneId::Network, Detail::Wired(_)) => self.opened_ethernet()?.connection,
            _ => None,
        }
    }

    pub(crate) fn set_ipv6_mode(&mut self, pane: PaneId, mode: usize) {
        match pane {
            PaneId::Wifi => self.wifi_ipv6 = mode,
            _ => self.ethernet_ipv6 = mode,
        }
        let change = match mode {
            IPV6_AUTOMATIC => Ipv6::Automatic,
            IPV6_OFF => Ipv6::Off,
            _ => return,
        };
        if let Some(path) = self.connection_path(pane) {
            self.commands
                .push(Command::Network(NetworkCommand::ChangeConnection(
                    pane,
                    path,
                    ConnectionChange::Ipv6(change),
                )));
        }
    }

    pub(crate) fn outside_rows(&mut self, outside: &Outside) -> Widgets {
        let mut status = Row::new(
            Text::new(&status_of(false, &outside.addressing), Tone::Primary),
            None,
        );
        if let Some(detail) = addressing_detail(&outside.addressing) {
            status = status.detail(detail);
        }
        let explanation = match &outside.configured_in {
            Some(file) => tr!(
                "{file} sets up {interface}, so Settings doesn’t change this connection.",
                file = file.display().to_string(),
                interface = outside.interface.clone()
            ),
            None => tr!(
                "{interface} is set up outside NetworkManager, for example in /etc/network/interfaces, so Settings doesn’t change this connection.",
                interface = outside.interface.clone()
            ),
        };
        let mut rows = vec![
            boxed(status),
            boxed(Row::new(
                Text::new(&explanation, Tone::Muted).caption(),
                None,
            )),
        ];
        let state = self
            .handover
            .clone()
            .filter(|handover| handover.interface() == outside.interface);
        let interface = outside.interface.clone();
        match state {
            None if outside.hand_over => {
                let mut ask = intent(&self.intents, move |()| {
                    Intent::HandOver(Some(HandOverCommand::Preview(interface.clone())))
                });
                let button =
                    Button::new(&tr!("Let Settings Manage This Connection…"), move || {
                        ask(())
                    });
                self.anchors.push((
                    Anchor::Item(format!("hand-over:{}", outside.interface)),
                    button.id(),
                ));
                rows.push(boxed(Row::new(
                    Text::new(
                        &tr!("NetworkManager takes over with the same settings, after you authenticate"),
                        Tone::Muted,
                    )
                    .caption(),
                    Some(boxed(button)),
                )));
            }
            None | Some(HandOver::Done(..)) => {}
            Some(HandOver::Asking(_)) => rows.push(muted_row(&tr!("Reading the setup…"))),
            Some(HandOver::Applying(_)) => {
                rows.push(muted_row(&tr!("Handing the connection to NetworkManager…")))
            }
            Some(HandOver::Refused(_, reason)) => rows.push(self.dismissable(tr!(
                "Settings can’t take this connection over: {reason}.",
                reason = reason
            ))),
            Some(HandOver::Failed(_, reason)) => {
                rows.push(self.dismissable(tr!("Nothing was changed: {reason}", reason = reason)))
            }
            Some(HandOver::Preview(_, changes)) => {
                rows.push(boxed(Row::new(
                    Text::new(
                        &tr!("Settings turns these lines into comments, keeps a copy of each file next to it and sets up the same connection in NetworkManager:"),
                        Tone::Primary,
                    ),
                    None,
                )));
                for change in &changes {
                    rows.push(boxed(
                        Row::new(
                            Text::new(&change.file.display().to_string(), Tone::Primary),
                            None,
                        )
                        .detail(Text::new(&change.lines.join("\n"), Tone::Muted).caption()),
                    ));
                }
                let mut cancel = intent(&self.intents, |()| Intent::HandOver(None));
                let mut apply = intent(&self.intents, move |()| {
                    Intent::HandOver(Some(HandOverCommand::Apply(interface.clone())))
                });
                let manage = Button::new(&tr!("Manage Connection"), move || apply(())).primary();
                self.anchors.push((
                    Anchor::Item(format!("hand-over:{}", outside.interface)),
                    manage.id(),
                ));
                rows.push(boxed(Row::new(
                    Text::new(&tr!("Other stanzas are left as they are"), Tone::Muted).caption(),
                    Some(boxed(Stack::new(
                        Axis::Horizontal,
                        Spacing::S as f32,
                        0.0,
                        vec![
                            boxed(Button::new(&tr!("Cancel"), move || cancel(())).cancel()),
                            boxed(manage),
                        ],
                    ))),
                )));
            }
        }
        rows
    }

    fn dismissable(&mut self, message: String) -> Box<dyn Widget> {
        let mut dismiss = intent(&self.intents, |()| Intent::HandOver(None));
        boxed(Row::new(
            Text::new(&message, Tone::Muted).caption(),
            Some(boxed(Button::new(&tr!("OK"), move || dismiss(())))),
        ))
    }

    pub(crate) fn handed_over_rows(&mut self) -> Widgets {
        let Some(HandOver::Done(interface, changes, connection)) = self.handover.clone() else {
            return Vec::new();
        };
        let mut rows = vec![boxed(Row::new(
            Text::new(
                &tr!(
                    "NetworkManager now manages {interface} with the connection “{connection}”.",
                    interface = interface,
                    connection = connection
                ),
                Tone::Primary,
            ),
            None,
        ))];
        for change in changes {
            let detail = match &change.backup {
                Some(backup) => tr!(
                    "Now comments: {lines}. The original is {backup}.",
                    lines = change.lines.join(" · "),
                    backup = backup.display().to_string()
                ),
                None => change.lines.join(" · "),
            };
            rows.push(boxed(
                Row::new(
                    Text::new(&change.file.display().to_string(), Tone::Primary),
                    None,
                )
                .detail(Text::new(&detail, Tone::Muted).caption()),
            ));
        }
        rows.push(self.dismissable(tr!("To undo this, put the original file back.")));
        rows
    }

    pub(crate) fn opened_ethernet(&self) -> Option<Ethernet> {
        let Some(Detail::Wired(interface)) = &self.detail else {
            return None;
        };
        self.network
            .as_ref()?
            .ethernets
            .iter()
            .find(|ethernet| ethernet.interface == *interface)
            .cloned()
    }

    pub(crate) fn ethernet_rows(&mut self) -> Widgets {
        let Some(network) = self.network.clone() else {
            return Vec::new();
        };
        let mut rows = self.handed_over_rows();
        if network.ethernets.is_empty() {
            rows.push(muted_row(&tr!("No Ethernet port")));
            return rows;
        }
        for ethernet in network.ethernets {
            let mut open = intent(&self.intents, Intent::OpenWired);
            let interface = ethernet.interface.clone();
            let button = Button::new(&tr!("Details…"), move || open(interface.clone()));
            let mut row = Row::new(
                Text::new(&ethernet.interface, Tone::Primary),
                Some(boxed(button)),
            )
            .detail(Text::new(&wired_status(&ethernet), Tone::Muted).caption());
            let mut activate = intent(&self.intents, Intent::OpenWired);
            let interface = ethernet.interface.clone();
            row = row.on_activate(move || activate(interface.clone()));
            rows.push(self.item_row(format!("wired:{}", ethernet.interface), row));
        }
        rows
    }

    pub(crate) fn open_wired(&mut self, interface: String) {
        self.detail = Some(Detail::Wired(interface));
        self.securing_wired = false;
        self.load_wired();
        self.clear_failure();
        self.focus_next = Some(Anchor::Back);
    }

    pub(crate) fn load_wired(&mut self) {
        let Some(ethernet) = self.opened_ethernet() else {
            return;
        };
        self.manual_ipv4 = ethernet.manual;
        let dns = match ethernet.manual {
            true => ethernet.addressing.dns.join(", "),
            false => ethernet
                .dns
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        };
        for (id, value) in [
            (
                SettingId::EthernetAddress,
                ethernet.addressing.address.clone(),
            ),
            (
                SettingId::EthernetRouter,
                ethernet.addressing.router.clone(),
            ),
            (SettingId::EthernetDns, Some(dns)),
        ] {
            self.values
                .insert(id, Value::Text(value.unwrap_or_default()));
        }
        self.ethernet_ipv6 = self.load_ipv6(ethernet.ipv6, &ETHERNET_IPV6_FIELDS);
        self.load_wired_security();
    }

    pub(crate) fn load_wired_security(&mut self) {
        self.enterprise = self
            .opened_ethernet()
            .and_then(|ethernet| ethernet.security)
            .unwrap_or_default();
        self.enterprise_file = self.enterprise.authority.is_some();
    }

    pub(crate) fn wired_page(&mut self) -> Option<Widgets> {
        let ethernet = self.opened_ethernet()?;
        let mut page: Widgets = self.back_button(PaneId::Network).into_iter().collect();
        let ink = self.palette().ink;
        let symbol = self.symbol(PaneId::Network.symbol(), HEADER_SYMBOL, ink);
        page.push(boxed(Header::new(
            symbol,
            Text::new(&ethernet.interface, Tone::Primary).role(TextRole::Title),
            &wired_status(&ethernet),
            None,
        )));
        if let Some(outside) = &ethernet.outside {
            let rows = self.outside_rows(outside);
            page.push(boxed(Card::new(rows)));
            return Some(page);
        }
        if let Some(detail) = addressing_detail(&ethernet.addressing) {
            page.push(boxed(Card::new(vec![boxed(
                Row::new(Text::new(&tr!("Addresses"), Tone::Primary), None).detail(detail),
            )])));
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
        let mut addressing = vec![boxed(Row::new(
            Text::new(&tr!("Configure IPv4"), Tone::Primary),
            Some(boxed(segmented)),
        ))];
        if self.manual_ipv4 {
            for (id, label, placeholder) in MANUAL_FIELDS {
                addressing.push(self.manual_field(id, label, placeholder));
            }
        } else if ethernet.connection.is_some() {
            addressing.push(self.manual_field(
                SettingId::EthernetDns,
                text!("DNS servers"),
                text!("Automatic"),
            ));
        }
        if ethernet.connection.is_some() {
            addressing.extend(self.ipv6_rows(
                PaneId::Network,
                self.ethernet_ipv6,
                &ETHERNET_IPV6_FIELDS,
            ));
        }
        page.push(boxed(Caption::new(Text::label(&tr!("TCP/IP")))));
        page.push(boxed(Card::new(addressing)));
        if ethernet.connection.is_some() {
            page.push(boxed(Caption::new(Text::label(&tr!("802.1X Security")))));
            let rows = self.wired_security_rows(&ethernet);
            page.push(boxed(Card::new(rows)));
        }
        Some(page)
    }

    fn wired_security_rows(&mut self, ethernet: &Ethernet) -> Widgets {
        if self.securing_wired {
            let mut rows = self.enterprise_rows(Intent::SaveWiredSecurity);
            let mut cancel = intent(&self.intents, |()| Intent::SecureWired(false));
            let detail = self
                .join
                .error
                .clone()
                .unwrap_or_else(|| tr!("Press Return in the password field to save"));
            rows.push(boxed(Row::new(
                Text::new(&detail, Tone::Muted).caption(),
                Some(boxed(
                    Button::new(&tr!("Cancel"), move || cancel(())).cancel(),
                )),
            )));
            return rows;
        }
        let mut change = intent(&self.intents, |()| Intent::SecureWired(true));
        let (status, action) = match &ethernet.security {
            Some(login) => (
                tr!(
                    "Signs in as {identity} with {method}",
                    identity = login.identity.clone(),
                    method = login.eap.label().to_owned()
                ),
                tr!("Change…"),
            ),
            None => (
                tr!("Off: the network doesn’t ask this computer to sign in"),
                tr!("Set Up…"),
            ),
        };
        let button = Button::new(&action, move || change(()));
        self.anchors
            .push((Anchor::Item("wired-security".to_owned()), button.id()));
        let mut controls = vec![boxed(button)];
        if ethernet.security.is_some() {
            let interface = ethernet.interface.clone();
            let mut off = intent(&self.intents, move |()| {
                Intent::Network(NetworkCommand::SecureEthernet(interface.clone(), None))
            });
            controls.insert(0, boxed(Button::new(&tr!("Turn Off"), move || off(()))));
        }
        vec![boxed(Row::new(
            Text::new(&status, Tone::Primary),
            Some(boxed(Stack::new(
                Axis::Horizontal,
                Spacing::S as f32,
                0.0,
                controls,
            ))),
        ))]
    }

    pub(crate) fn save_wired_security(&mut self, password: String) {
        let Some(ethernet) = self.opened_ethernet() else {
            return;
        };
        let Some(login) = self.enterprise_login(password) else {
            return;
        };
        self.join.error = None;
        self.securing_wired = false;
        self.commands
            .push(Command::Network(NetworkCommand::SecureEthernet(
                ethernet.interface,
                Some(Box::new(login)),
            )));
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
        let mut button = Button::new(&tr!("Import Configuration…"), move || import(()));
        button.set_enabled(!self.installing_vpn_plugin);
        self.anchors
            .push((Anchor::Setting(SettingId::Vpns, None), button.id()));
        let hint = match self.installing_vpn_plugin {
            true => tr!("Installing OpenVPN support…"),
            false => tr!("Add a WireGuard (.conf) or OpenVPN (.ovpn) file"),
        };
        rows.push(boxed(Row::new(
            Text::new(&hint, Tone::Muted).caption(),
            Some(boxed(button)),
        )));
        rows
    }

    pub(crate) fn proxy_rows(&mut self) -> Widgets {
        let Some(proxy) = self.proxy.clone() else {
            return Vec::new();
        };
        let labels = [tr!("None"), tr!("Manual")];
        let selected = ProxyMode::ALL.iter().position(|mode| *mode == proxy.mode);
        let segmented = SegmentedControl::new(
            &tr!("Proxy"),
            &labels,
            selected,
            intent(&self.intents, Intent::ProxyMode),
        );
        self.anchors
            .push((Anchor::Setting(SettingId::Proxy, None), segmented.id()));
        let mut rows = vec![boxed(Row::new(
            Text::new(&tr!("Proxy"), Tone::Primary),
            Some(boxed(segmented)),
        ))];
        match proxy.mode {
            ProxyMode::None => {}
            ProxyMode::Manual => {
                for (id, label, placeholder) in PROXY_FIELDS {
                    rows.push(self.manual_field(id, label, placeholder));
                }
            }
        }
        if proxy.mode != ProxyMode::None {
            rows.push(boxed(Row::new(
                Text::new(
                    &tr!("Apps you open after logging out and back in use the proxy"),
                    Tone::Muted,
                )
                .caption(),
                None,
            )));
        }
        rows
    }

    pub(crate) fn load_proxy(&mut self, proxy: ProxySettings) {
        for (id, value) in [
            (SettingId::ProxyHttp, &proxy.http),
            (SettingId::ProxyHttps, &proxy.https),
            (SettingId::ProxyIgnore, &proxy.bypass),
        ] {
            self.values.insert(id, Value::Text(value.clone()));
        }
        self.proxy = Some(proxy);
    }

    pub(crate) fn set_proxy_mode(&mut self, index: usize) {
        let Some(mode) = ProxyMode::ALL.get(index).copied() else {
            return;
        };
        let Some(proxy) = &mut self.proxy else {
            return;
        };
        proxy.mode = mode;
        if mode == ProxyMode::None {
            self.commands.push(Command::Proxy(ProxySettings::default()));
        }
    }

    pub(crate) fn change_proxy(&mut self) {
        let Some(mode) = self.proxy.as_ref().map(|proxy| proxy.mode) else {
            return;
        };
        let settings = ProxySettings {
            mode,
            http: text_of(self, SettingId::ProxyHttp),
            https: text_of(self, SettingId::ProxyHttps),
            bypass: text_of(self, SettingId::ProxyIgnore),
        };
        let ready = match mode {
            ProxyMode::None => true,
            ProxyMode::Manual => !settings.http.trim().is_empty(),
        };
        if ready {
            self.clear_failure();
            self.commands.push(Command::Proxy(settings));
        }
    }
}
