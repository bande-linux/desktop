use bande_keyboard::{Action, SHORTCUTS};
use bande_toolkit::ui::{Menu, MenuItem};

use bande_status::Command;
use bande_status::dbusmenu::{Node, command};
use bande_status::status::{
    Battery, Bluetooth, Capture, InputSources, Power, Sound, Wifi, hours_and_minutes,
};

use crate::L10N;
use crate::rail::in_use;

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

pub fn clock_menu(quiet: bool) -> Menu {
    Menu::new(vec![
        item(
            &L10N.tr("do-not-disturb"),
            &Command::SetDoNotDisturb(!quiet),
        )
        .checked(quiet),
        MenuItem::separator(),
        settings("notifications"),
        settings("language-time"),
    ])
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
        item(&L10N.tr("lock-screen"), &Command::LockScreen).shortcut("Super+L"),
        item(
            &L10N.tr_args("log-out", [("user", user.to_owned().into())]),
            &Command::LogOut,
        ),
    ])
}

pub fn battery_menu(battery: &Battery, percentage_shown: bool) -> Menu {
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
    items.push(MenuItem::separator());
    items.push(
        item(
            &L10N.tr("show-battery-percentage"),
            &Command::ShowBatteryPercentage(!percentage_shown),
        )
        .checked(percentage_shown),
    );
    Menu::new(items)
}

pub fn wifi_menu(wifi: &Wifi) -> Menu {
    let mut items =
        vec![item(&L10N.tr("wifi"), &Command::SetWifi(!wifi.enabled)).checked(wifi.enabled)];
    if wifi.sign_in {
        items.push(MenuItem::separator());
        items.push(note(&L10N.tr("sign-in-required")));
        items.push(item(&L10N.tr("sign-in"), &Command::SignIn));
    }
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

pub fn sound_menu(sound: &Sound, capturing: &[Capture]) -> Menu {
    let percent = [("percent", (f64::from(sound.volume) * 100.0).round().into())];
    let mut items = Vec::new();
    let in_use = in_use(capturing);
    if !in_use.is_empty() {
        items.extend(in_use.iter().map(|line| note(line)));
        items.push(MenuItem::separator());
    }
    items.extend([
        note(&L10N.tr_args("volume-level", percent)),
        item(&L10N.tr("mute"), &Command::SetMuted(!sound.muted)).checked(sound.muted),
    ]);
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
            let title = match gadget.battery {
                Some(percent) => L10N.tr_args(
                    "device-battery",
                    [
                        ("name", gadget.name.clone().into()),
                        ("percent", percent.into()),
                    ],
                ),
                None => gadget.name.clone(),
            };
            items.push(
                item(&title, &Command::ToggleGadget(gadget.address.clone()))
                    .checked(gadget.connected),
            );
        }
        let forgettable = bluetooth
            .devices
            .iter()
            .map(|gadget| item(&gadget.name, &Command::ForgetGadget(gadget.address.clone())))
            .collect();
        items.push(MenuItem::submenu(
            &L10N.tr("forget-device"),
            Menu::new(forgettable),
        ));
    }
    items.push(MenuItem::separator());
    items.push(settings("bluetooth"));
    Menu::new(items)
}

pub fn input_source_menu(input: &InputSources) -> Menu {
    let mut items: Vec<MenuItem> = input
        .sources
        .iter()
        .enumerate()
        .map(|(index, source)| {
            item(&source.name, &Command::SelectInputSource(index)).checked(index == input.active)
        })
        .collect();
    items.push(MenuItem::separator());
    items.push(settings("keyboard"));
    Menu::new(items)
}

pub fn app_menu(node: &Node, item: &str) -> Menu {
    Menu::new(
        node.children
            .iter()
            .filter(|child| child.visible)
            .map(|child| {
                if child.separator {
                    return MenuItem::separator();
                }
                if child.children.is_empty() {
                    let entry = MenuItem::new(&child.label, &command(child, item).action())
                        .enabled(child.enabled)
                        .checked(child.checked);
                    match &child.shortcut {
                        Some(shortcut) => entry.shortcut(shortcut),
                        None => entry,
                    }
                } else {
                    MenuItem::submenu(&child.label, app_menu(child, item)).enabled(child.enabled)
                }
            })
            .collect(),
    )
}

pub struct AppMenus<'a> {
    pub name: &'a str,
    pub root: Option<&'a Node>,
    pub item: &'a str,
    pub actions: &'a [(String, String)],
    pub windows: &'a [(String, String)],
}

fn standard_app_menu(name: &str) -> Menu {
    let app = || [("app", name.to_owned().into())];
    Menu::new(vec![
        MenuItem::new(&L10N.tr_args("app-about", app()), "").enabled(false),
        MenuItem::separator(),
        item(&L10N.tr_args("app-hide", app()), &Command::HideApp),
        item(&L10N.tr("app-hide-others"), &Command::HideOthers),
        item(&L10N.tr("app-show-all"), &Command::ShowAll),
        MenuItem::separator(),
        item(&L10N.tr_args("app-quit", app()), &Command::QuitApp).shortcut("Ctrl+Q"),
    ])
}

pub fn cascade(menus: &AppMenus<'_>) -> Menu {
    let exported = menus.root.and_then(|root| root.children.first());
    let app_row = match exported {
        Some(node) => app_menu(node, menus.item),
        None => standard_app_menu(menus.name),
    };
    let mut items = vec![MenuItem::submenu(menus.name, app_row).strong()];
    match menus.root {
        Some(root) => {
            for child in root
                .children
                .iter()
                .skip(1)
                .filter(|child| child.visible && !child.separator)
            {
                items.push(if child.children.is_empty() {
                    MenuItem::new(&child.label, &command(child, menus.item).action())
                        .enabled(child.enabled)
                } else {
                    MenuItem::submenu(&child.label, app_menu(child, menus.item))
                        .enabled(child.enabled)
                });
            }
        }
        None => {
            if !menus.actions.is_empty() {
                items.push(MenuItem::separator());
            }
            for (id, title) in menus.actions {
                items.push(item(title, &Command::AppAction(id.clone())));
            }
            if !menus.windows.is_empty() {
                items.push(MenuItem::separator());
            }
            for (identifier, title) in menus.windows {
                items.push(item(title, &Command::ShowWindow(identifier.clone())));
            }
        }
    }
    let shortcut = SHORTCUTS
        .iter()
        .find(|shortcut| shortcut.action == Action::SearchMenus)
        .map_or("", |shortcut| shortcut.keys);
    Menu::new(items).searchable(
        &L10N.tr("type-to-search"),
        shortcut,
        &L10N.tr("no-menu-results"),
    )
}
