use std::rc::Rc;

use unbound_network::VpnState;
use unbound_settings_panes::{Group, Pane, PaneId, SettingId, Value};
use unbound_toolkit::ui::{Toggle, Widget};
use unbound_toolkit::{Color, Image, TextRole};
use unbound_xdg::{ICON_THEMES, find_icon};

use crate::L10N;
use crate::pages::{Widgets, boxed};
use crate::power::charge_text;
use crate::software::SoftwareState;
use crate::time::zone_place;
use crate::view::{Intent, SettingsView, intent};
use crate::widgets::{
    Column, HEADER_SYMBOL, Header, NOTICE_SYMBOL, Notice, SIDEBAR_SYMBOL, Scroll, SidebarLabel,
    SidebarRow, Text, Tone,
};

const SYMBOL_DENSITY: f32 = 2.0;
const ATTENTION_SYMBOL: &str = "view-refresh-symbolic";
const SEARCH_SYMBOL: &str = "system-search-symbolic";

impl SettingsView {
    pub(crate) fn symbol(&mut self, name: &'static str, size: f32, color: Color) -> Option<Image> {
        let pixels = (size * SYMBOL_DENSITY) as u32;
        let key = (name, pixels, [color.r, color.g, color.b, color.a]);
        let dirs = &self.icon_dirs;
        self.symbols
            .entry(key)
            .or_insert_with(|| {
                find_icon(name, pixels, &ICON_THEMES, dirs)
                    .and_then(|path| Image::load(&path, pixels))
                    .and_then(|image| image.tinted(color))
            })
            .clone()
    }

    fn palette(&mut self) -> unbound_toolkit::Palette {
        self.ui().context().palette
    }

    pub(crate) fn attention(&self) -> Option<String> {
        match &self.software {
            SoftwareState::Available(packages) if !packages.is_empty() => {
                Some(L10N.tr_args("updates-available", [("count", packages.len().into())]))
            }
            SoftwareState::Installed { restart: true } => Some(L10N.tr("restart-to-update")),
            _ => None,
        }
    }

    fn pane_name(&self, id: PaneId) -> String {
        self.panes
            .iter()
            .find(|pane| pane.id == id)
            .map(|pane| pane.name.clone())
            .unwrap_or_default()
    }

    fn sidebar_row(&mut self, id: PaneId, label: Text, searching: bool) -> Box<dyn Widget> {
        let selected = id == self.pane || (!searching && self.pane.parent() == Some(id));
        let palette = self.palette();
        let tint = if selected {
            palette.accent
        } else {
            palette.ink2
        };
        let symbol = self.symbol(id.symbol(), SIDEBAR_SYMBOL, tint);
        let mut select = intent(&self.intents, Intent::SelectPane);
        let row = SidebarRow::new(
            label,
            symbol,
            selected,
            move || select(id),
            intent(&self.intents, Intent::StepPane),
        );
        self.anchors.push((crate::view::Anchor::Pane(id), row.id()));
        boxed(row)
    }

    pub(crate) fn sidebar(&mut self) -> Box<dyn Widget> {
        let visible = self.visible_panes();
        let searching = !self.query.trim().is_empty();
        let mut rows: Widgets = Vec::new();
        if !searching && let Some(attention) = self.attention() {
            let accent = self.palette().accent;
            let symbol = self.symbol(ATTENTION_SYMBOL, SIDEBAR_SYMBOL, accent);
            let mut open = intent(&self.intents, |()| {
                Intent::SelectPane(PaneId::SoftwareUpdate)
            });
            let row = SidebarRow::new(
                Text::new(&attention, Tone::Primary).semibold(),
                symbol,
                self.pane == PaneId::SoftwareUpdate,
                move || open(()),
                intent(&self.intents, Intent::StepPane),
            );
            rows.push(boxed(row));
        }
        let mut group: Option<Group> = None;
        for id in visible {
            if !searching && id.group() != group {
                group = id.group();
                if let Some(group) = group {
                    rows.push(boxed(SidebarLabel::new(&group.label())));
                }
            }
            let highlights = self.hit(id).map(|hit| hit.name.clone()).unwrap_or_default();
            let name = self.pane_name(id);
            let label = match id.parent().filter(|_| searching) {
                Some(parent) => {
                    let path = format!("{} › ", self.pane_name(parent));
                    let shifted = highlights
                        .into_iter()
                        .map(|range| range.start + path.len()..range.end + path.len())
                        .collect();
                    Text::new(&format!("{path}{name}"), Tone::Primary).highlighted(shifted)
                }
                None => Text::new(&name, Tone::Primary).highlighted(highlights),
            };
            let row = self.sidebar_row(id, label, searching);
            rows.push(row);
        }
        boxed(Scroll::new(
            boxed(Column::new(2.0, 0.0, rows)),
            Rc::clone(&self.sidebar_scroll),
        ))
    }

    pub(crate) fn summary(&self, pane: &Pane) -> String {
        let live = match pane.id {
            PaneId::Wifi => self.wifi.as_ref().map(|wifi| {
                match wifi.networks.iter().find(|network| network.current) {
                    _ if !wifi.enabled => L10N.tr("off"),
                    Some(network) => {
                        L10N.tr_args("connected-to", [("network", network.name().into())])
                    }
                    None => L10N.tr("not-connected"),
                }
            }),
            PaneId::Bluetooth => self.bluetooth.as_ref().map(|bluetooth| {
                let connected = bluetooth
                    .paired
                    .iter()
                    .filter(|device| device.connected)
                    .count();
                match (bluetooth.powered, connected) {
                    (false, _) => L10N.tr("off"),
                    (true, 0) => L10N.tr("on"),
                    (true, count) => {
                        L10N.tr_args("on-devices-connected", [("count", count.into())])
                    }
                }
            }),
            PaneId::Network => self.network.as_ref().and_then(|network| {
                let mut parts: Vec<String> = network
                    .ethernet
                    .iter()
                    .map(|ethernet| match (ethernet.connected, ethernet.cable) {
                        (true, _) => L10N.tr("ethernet-connected"),
                        (false, true) => L10N.tr("ethernet-not-connected"),
                        (false, false) => L10N.tr("no-cable"),
                    })
                    .collect();
                parts.extend(
                    network
                        .vpns
                        .iter()
                        .filter(|vpn| vpn.state == VpnState::Connected)
                        .map(|vpn| {
                            L10N.tr_args("vpn-connected", [("vpn", vpn.name.clone().into())])
                        }),
                );
                (!parts.is_empty()).then(|| parts.join(" · "))
            }),
            PaneId::Notifications => (self.values.get(&SettingId::DoNotDisturb)
                == Some(&Value::Flag(true)))
            .then(|| L10N.tr("do-not-disturb-on")),
            PaneId::Displays => match self.displays.as_slice() {
                [] => None,
                [display] => Some(display.name()),
                displays => Some(L10N.tr_args("display-count", [("count", displays.len().into())])),
            },
            PaneId::Sound => self.audio.as_ref().and_then(|audio| {
                let output = audio.default_output()?;
                let percent = (output.level.volume * 100.0).round();
                Some(match output.level.muted {
                    true => L10N.tr_args(
                        "output-muted",
                        [("output", output.description.clone().into())],
                    ),
                    false => L10N.tr_args(
                        "output-level",
                        [
                            ("output", output.description.clone().into()),
                            ("percent", percent.into()),
                        ],
                    ),
                })
            }),
            PaneId::Keyboard => {
                let sources = self.input_sources();
                let name = sources
                    .first()
                    .zip(self.layouts.as_ref())
                    .map(|(first, layouts)| layouts.name(first));
                name.map(|name| match sources.len() {
                    1 => name,
                    count => L10N.tr_args(
                        "input-source-count",
                        [("source", name.into()), ("count", count.into())],
                    ),
                })
            }
            PaneId::Power => self.battery.as_ref().map(|battery| {
                L10N.tr_args(
                    "battery-summary",
                    [
                        ("percent", battery.percent.round().into()),
                        ("charge", charge_text(battery.charge).into()),
                    ],
                )
            }),
            PaneId::LanguageTime => {
                let language = self.locale.as_ref().and_then(|locale| {
                    locale
                        .installed
                        .iter()
                        .find(|installed| installed.code == locale.language)
                        .map(|installed| installed.name.clone())
                });
                let place = self.time.as_ref().map(|time| zone_place(&time.zone).0);
                let parts: Vec<String> = language.into_iter().chain(place).collect();
                (!parts.is_empty()).then(|| parts.join(" · "))
            }
            PaneId::About => self.about.as_ref().map(|about| about.model.clone()),
            PaneId::Storage | PaneId::SoftwareUpdate => self.pane_summary(pane.id),
            PaneId::Appearance
            | PaneId::Bar
            | PaneId::Accessibility
            | PaneId::KeyboardShortcuts
            | PaneId::TrackpadMouse
            | PaneId::Apps
            | PaneId::PrivacySecurity => None,
        };
        live.unwrap_or_else(|| pane.summary.clone())
    }

    pub(crate) fn header(&mut self, pane: &Pane) -> Box<dyn Widget> {
        let ink = self.palette().ink;
        let symbol = self.symbol(pane.id.symbol(), HEADER_SYMBOL, ink);
        let title = match (pane.id, &self.about) {
            (PaneId::About, Some(about)) if !about.name.is_empty() => {
                Text::new(&about.name, Tone::Primary).role(TextRole::Display)
            }
            _ => Text::new(&pane.name, Tone::Primary).role(TextRole::Title),
        };
        let master = pane.master.and_then(|id| {
            let Some(Value::Flag(on)) = self.values.get(&id) else {
                return None;
            };
            let setting = pane.settings().find(|setting| setting.id == id)?;
            let set = intent(&self.intents, move |on| Intent::Set(id, Value::Flag(on)));
            let toggle = Toggle::new(&setting.label, *on, set);
            self.anchors
                .push((crate::view::Anchor::Setting(id, None), toggle.id()));
            Some(boxed(toggle))
        });
        boxed(Header::new(symbol, title, &self.summary(pane), master))
    }

    pub(crate) fn no_match(&mut self) -> Box<dyn Widget> {
        let faint = self.palette().ink3;
        let symbol = self.symbol(SEARCH_SYMBOL, NOTICE_SYMBOL, faint);
        let heading = L10N.tr_args("no-match", [("query", self.query.trim().to_owned().into())]);
        boxed(Notice::new(symbol, &heading, &L10N.tr("no-match-hint")))
    }

    pub(crate) fn notice(&mut self, pane: &Pane) -> Option<Box<dyn Widget>> {
        let name = pane.name.clone();
        let (heading, line) = if !self.available(pane.id) {
            (
                L10N.tr_args("unavailable", [("pane", name.clone().into())]),
                L10N.tr_args("unavailable-hint", [("pane", name.into())]),
            )
        } else if let Some(master) = pane.master
            && self.values.get(&master) == Some(&Value::Flag(false))
        {
            (
                L10N.tr_args("pane-off", [("pane", name.into())]),
                L10N.tr(&format!("{}-off-hint", pane.id.keyword())),
            )
        } else {
            return None;
        };
        let faint = self.palette().ink3;
        let symbol = self.symbol(pane.id.symbol(), NOTICE_SYMBOL, faint);
        Some(boxed(Notice::new(symbol, &heading, &line)))
    }
}
