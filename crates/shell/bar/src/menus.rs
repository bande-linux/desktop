use unbound_toolkit::ui::{Menu, MenuItem};

use unbound_status::Command;
use unbound_status::dbusmenu::{Node, command};
use unbound_status::status::{
    Battery, Bluetooth, InputSources, Power, Sound, Wifi, hours_and_minutes,
};

use crate::L10N;

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
        item(&L10N.tr_args("app-hide", app()), &Command::HideApp).shortcut("Super+H"),
        item(&L10N.tr("app-hide-others"), &Command::HideOthers).shortcut("Alt+Super+H"),
        item(&L10N.tr("app-show-all"), &Command::ShowAll),
        MenuItem::separator(),
        item(&L10N.tr_args("app-quit", app()), &Command::QuitApp).shortcut("Super+Q"),
    ])
}

pub fn cascade(menus: &AppMenus<'_>) -> Menu {
    let exported = menus.root.and_then(|root| root.children.first());
    let app_row = match exported {
        Some(node) => app_menu(node, menus.item),
        None => standard_app_menu(menus.name),
    };
    let mut items = vec![MenuItem::submenu(menus.name, app_row)];
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
    Menu::new(items).searchable(&L10N.tr("search-menus"), &L10N.tr("no-menu-results"))
}
