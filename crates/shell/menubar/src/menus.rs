use unbound_toolkit::ui::{Menu, MenuItem};

use crate::L10N;
use crate::status::{Battery, Bluetooth, Power, Sound, Wifi, hours_and_minutes};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    About,
    Settings(Option<String>),
    ForceQuit,
    Sleep,
    Restart,
    ShutDown,
    LockScreen,
    LogOut,
    SetWifi(bool),
    Join(Vec<u8>),
    SetMuted(bool),
    UseOutput(String),
    SetBluetooth(bool),
    ToggleGadget(String),
    Item { item: String, id: i32 },
    AboutApp(String),
    HideApp,
    HideOthers,
    ShowAll,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect()
}

fn switch(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

impl Command {
    pub fn action(&self) -> String {
        match self {
            Self::About => "about".to_owned(),
            Self::Settings(None) => "settings".to_owned(),
            Self::Settings(Some(pane)) => format!("settings:{pane}"),
            Self::ForceQuit => "force-quit".to_owned(),
            Self::Sleep => "sleep".to_owned(),
            Self::Restart => "restart".to_owned(),
            Self::ShutDown => "shut-down".to_owned(),
            Self::LockScreen => "lock".to_owned(),
            Self::LogOut => "log-out".to_owned(),
            Self::SetWifi(on) => format!("wifi:{}", switch(*on)),
            Self::Join(ssid) => format!("join:{}", hex(ssid)),
            Self::SetMuted(on) => format!("mute:{}", switch(*on)),
            Self::UseOutput(name) => format!("output:{name}"),
            Self::SetBluetooth(on) => format!("bluetooth:{}", switch(*on)),
            Self::ToggleGadget(address) => format!("gadget:{address}"),
            Self::Item { item, id } => format!("item:{item}|{id}"),
            Self::AboutApp(version) => format!("about-app:{version}"),
            Self::HideApp => "hide-app".to_owned(),
            Self::HideOthers => "hide-others".to_owned(),
            Self::ShowAll => "show-all".to_owned(),
        }
    }

    pub fn parse(action: &str) -> Option<Self> {
        let (kind, argument) = action.split_once(':').unwrap_or((action, ""));
        let on = || match argument {
            "on" => Some(true),
            "off" => Some(false),
            _ => None,
        };
        Some(match kind {
            "about" => Self::About,
            "settings" => Self::Settings(Some(argument.to_owned()).filter(|pane| !pane.is_empty())),
            "force-quit" => Self::ForceQuit,
            "sleep" => Self::Sleep,
            "restart" => Self::Restart,
            "shut-down" => Self::ShutDown,
            "lock" => Self::LockScreen,
            "log-out" => Self::LogOut,
            "wifi" => Self::SetWifi(on()?),
            "join" => Self::Join(unhex(argument)?),
            "mute" => Self::SetMuted(on()?),
            "output" => Self::UseOutput(argument.to_owned()),
            "bluetooth" => Self::SetBluetooth(on()?),
            "gadget" => Self::ToggleGadget(argument.to_owned()),
            "about-app" => Self::AboutApp(argument.to_owned()),
            "hide-app" => Self::HideApp,
            "hide-others" => Self::HideOthers,
            "show-all" => Self::ShowAll,
            "item" => {
                let (item, id) = argument.rsplit_once('|')?;
                Self::Item {
                    item: item.to_owned(),
                    id: id.parse().ok()?,
                }
            }
            _ => return None,
        })
    }
}

fn item(title: &str, command: &Command) -> MenuItem {
    MenuItem::new(title, &command.action())
}

fn note(title: &str) -> MenuItem {
    MenuItem::new(title, "").enabled(false)
}

fn settings(pane: &str) -> MenuItem {
    item(
        &L10N.tr(&format!("{pane}-settings")),
        &Command::Settings(Some(pane.to_owned())),
    )
}

pub fn system_menu(user: &str) -> Menu {
    Menu::new(vec![
        item(&L10N.tr("about"), &Command::About),
        MenuItem::separator(),
        item(&L10N.tr("system-settings"), &Command::Settings(None)),
        MenuItem::separator(),
        item(&L10N.tr("force-quit"), &Command::ForceQuit),
        MenuItem::separator(),
        item(&L10N.tr("sleep"), &Command::Sleep),
        item(&L10N.tr("restart"), &Command::Restart),
        item(&L10N.tr("shut-down"), &Command::ShutDown),
        MenuItem::separator(),
        item(&L10N.tr("lock-screen"), &Command::LockScreen).shortcut("Ctrl+Super+Q"),
        item(
            &L10N.tr_args("log-out", [("user", user.to_owned().into())]),
            &Command::LogOut,
        ),
    ])
}

pub fn battery_menu(battery: &Battery) -> Menu {
    let percent = [("percent", battery.percent.round().into())];
    let mut items = vec![note(&L10N.tr_args("battery-level", percent))];
    let estimate = match battery.power {
        Power::Full => L10N.tr("battery-full"),
        Power::Battery {
            remaining: Some(remaining),
        } => L10N.tr_args(
            "battery-remaining",
            [("time", hours_and_minutes(remaining).into())],
        ),
        Power::Charging {
            until_full: Some(until_full),
        } => L10N.tr_args(
            "battery-until-full",
            [("time", hours_and_minutes(until_full).into())],
        ),
        Power::Battery { remaining: None } | Power::Charging { until_full: None } => {
            L10N.tr("battery-estimating")
        }
    };
    items.push(note(&estimate));
    items.push(MenuItem::separator());
    items.push(note(&if battery.on_adapter() {
        L10N.tr("power-source-adapter")
    } else {
        L10N.tr("power-source-battery")
    }));
    Menu::new(items)
}

pub fn wifi_menu(wifi: &Wifi) -> Menu {
    let mut items =
        vec![item(&L10N.tr("wifi"), &Command::SetWifi(!wifi.enabled)).checked(wifi.enabled)];
    if wifi.enabled {
        let mut networks: Vec<_> = wifi.networks.iter().collect();
        networks.sort_by(|a, b| {
            (b.current, b.known, b.strength).cmp(&(a.current, a.known, a.strength))
        });
        let known: Vec<_> = networks.iter().filter(|network| network.known).collect();
        let other: Vec<_> = networks.iter().filter(|network| !network.known).collect();
        for (heading, group) in [("known-networks", known), ("other-networks", other)] {
            if group.is_empty() {
                continue;
            }
            items.push(MenuItem::separator());
            items.push(note(&L10N.tr(heading)));
            for network in group {
                let mut title = network.name();
                if network.security.needs_password() && !network.known {
                    title = L10N.tr_args("secured-network", [("name", title.into())]);
                }
                items.push(
                    item(&title, &Command::Join(network.ssid.clone())).checked(network.current),
                );
            }
        }
    }
    items.push(MenuItem::separator());
    items.push(settings("wifi"));
    Menu::new(items)
}

pub fn sound_menu(sound: &Sound) -> Menu {
    let percent = [("percent", (f64::from(sound.volume) * 100.0).round().into())];
    let mut items = vec![
        note(&L10N.tr_args("volume-level", percent)),
        item(&L10N.tr("mute"), &Command::SetMuted(!sound.muted)).checked(sound.muted),
    ];
    if !sound.outputs.is_empty() {
        items.push(MenuItem::separator());
        items.push(note(&L10N.tr("output")));
        for output in &sound.outputs {
            items.push(
                item(
                    &output.description,
                    &Command::UseOutput(output.name.clone()),
                )
                .checked(sound.default_output.as_deref() == Some(output.name.as_str())),
            );
        }
    }
    items.push(MenuItem::separator());
    items.push(settings("sound"));
    Menu::new(items)
}

pub fn bluetooth_menu(bluetooth: &Bluetooth) -> Menu {
    let mut items = vec![
        item(
            &L10N.tr("bluetooth"),
            &Command::SetBluetooth(!bluetooth.powered),
        )
        .checked(bluetooth.powered),
    ];
    if bluetooth.powered && !bluetooth.devices.is_empty() {
        items.push(MenuItem::separator());
        items.push(note(&L10N.tr("devices")));
        for gadget in &bluetooth.devices {
            items.push(
                item(&gadget.name, &Command::ToggleGadget(gadget.address.clone()))
                    .checked(gadget.connected),
            );
        }
    }
    items.push(MenuItem::separator());
    items.push(settings("bluetooth"));
    Menu::new(items)
}
