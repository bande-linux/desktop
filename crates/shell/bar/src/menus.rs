use bande_datetime::DateFormats;
use bande_keyboard::{Action, keys_of};
use bande_l10n::{text, tr};
use bande_toolkit::ui::{Menu, MenuItem};
use jiff::{Timestamp, Zoned};

use crate::rail::in_use;
use bande_status::Command;
use bande_status::clock_app::ClockApp;
use bande_status::dbusmenu::{Node, command};
use bande_status::status::{
    Battery, Bluetooth, Capture, InputSources, Power, Sound, Wifi, hours_and_minutes,
};

fn item(title: &str, command: &Command) -> MenuItem {
    MenuItem::new(title, &command.action())
}

fn note(title: &str) -> MenuItem {
    MenuItem::new(title, "").enabled(false)
}

fn settings(title: String, pane: &str) -> MenuItem {
    item(&title, &Command::Settings(Some(pane.to_owned())))
}

fn clock_time(dates: &DateFormats, at: &Zoned) -> String {
    let (time, period) = dates.time(at);
    period.map_or(time.clone(), |period| format!("{time} {period}"))
}

fn clock_items(clock: &ClockApp, dates: &DateFormats, has_clock: bool) -> Vec<MenuItem> {
    let now = bande_datetime::now();
    let local = |seconds: i64| {
        Timestamp::from_second(seconds)
            .ok()
            .map(|at| at.to_zoned(now.time_zone().clone()))
    };
    let mut items: Vec<MenuItem> = clock
        .timers
        .iter()
        .filter_map(|(name, ends)| {
            let ends = local(*ends)?;
            let time = clock_time(dates, &ends);
            Some(note(&tr!(
                "{name} · ends at {time}",
                name = name.clone(),
                time = time
            )))
        })
        .collect();
    if let Some((name, at)) = &clock.next_alarm
        && let Some(at) = local(*at)
    {
        let time = match at.date() == now.date() {
            true => clock_time(dates, &at),
            false => format!(
                "{} {}",
                dates.short_weekday(at.date()),
                clock_time(dates, &at)
            ),
        };
        items.push(note(&tr!(
            "{name} · {time}",
            name = name.clone(),
            time = time
        )));
    }
    if has_clock {
        items.push(item(&tr!("Open Clock"), &Command::OpenClock));
    }
    if !items.is_empty() {
        items.push(MenuItem::separator());
    }
    items
}

pub fn clock_menu(quiet: bool, clock: &ClockApp, dates: &DateFormats, has_clock: bool) -> Menu {
    let mut items = clock_items(clock, dates, has_clock);
    items.extend([
        item(&tr!("Do Not Disturb"), &Command::SetDoNotDisturb(!quiet)).checked(quiet),
        MenuItem::separator(),
        settings(tr!("Notifications Settings…"), "notifications"),
        settings(tr!("Date & Time Settings…"), "language-time"),
    ]);
    Menu::new(items)
}

pub fn system_menu(user: &str, frontmost: Option<&str>, can_sleep: bool) -> Menu {
    let force_quit = item(&tr!("Force Quit…"), &Command::ForceQuit);
    let force_quit = match frontmost {
        Some(app) => force_quit.shift_alternate(item(
            &tr!("Force Quit “{app}”", app = app.to_owned()),
            &Command::ForceQuitApp,
        )),
        None => force_quit,
    };
    let mut items = vec![
        item(&tr!("About This Computer"), &Command::About),
        MenuItem::separator(),
        item(&tr!("System Settings…"), &Command::Settings(None)),
        MenuItem::separator(),
        force_quit,
        MenuItem::separator(),
    ];
    if can_sleep {
        items.push(item(&tr!("Sleep"), &Command::Sleep));
    }
    items.extend([
        item(&tr!("Restart…"), &Command::Restart),
        item(&tr!("Shut Down…"), &Command::ShutDown),
        MenuItem::separator(),
        item(&tr!("Lock Screen"), &Command::LockScreen)
            .shortcut(&keys_of(Action::LockScreen).unwrap_or_default()),
        item(
            &tr!("Log Out {user}…", user = user.to_owned()),
            &Command::LogOut,
        ),
    ]);
    Menu::new(items)
}

pub fn battery_menu(battery: &Battery, percentage_shown: bool) -> Menu {
    let mut items = vec![note(&tr!(
        "Battery {percent}%",
        percent = battery.percent.round()
    ))];
    let estimate = match battery.power {
        Power::Full => tr!("Fully Charged"),
        Power::Battery {
            remaining: Some(remaining),
        } => tr!("{time} remaining", time = hours_and_minutes(remaining)),
        Power::Charging {
            until_full: Some(until_full),
        } => tr!("{time} until full", time = hours_and_minutes(until_full)),
        Power::Battery { remaining: None } | Power::Charging { until_full: None } => {
            tr!("Estimating…")
        }
    };
    items.push(note(&estimate));
    items.push(MenuItem::separator());
    items.push(note(&if battery.on_adapter() {
        tr!("Power Source: Power Adapter")
    } else {
        tr!("Power Source: Battery")
    }));
    items.push(MenuItem::separator());
    items.push(
        item(
            &tr!("Show Percentage"),
            &Command::ShowBatteryPercentage(!percentage_shown),
        )
        .checked(percentage_shown),
    );
    Menu::new(items)
}

pub fn wifi_menu(wifi: &Wifi) -> Menu {
    let mut items =
        vec![item(&tr!("Wi-Fi"), &Command::SetWifi(!wifi.enabled)).checked(wifi.enabled)];
    if wifi.sign_in {
        items.push(MenuItem::separator());
        items.push(note(&tr!("Sign in required")));
        items.push(item(&tr!("Sign In…"), &Command::SignIn));
    }
    if wifi.enabled {
        let mut networks: Vec<_> = wifi.networks.iter().collect();
        networks.sort_by(|a, b| {
            (b.current, b.known, b.strength).cmp(&(a.current, a.known, a.strength))
        });
        let known: Vec<_> = networks.iter().filter(|network| network.known).collect();
        let other: Vec<_> = networks.iter().filter(|network| !network.known).collect();
        for (heading, group) in [
            (text!("Known Networks"), known),
            (text!("Other Networks"), other),
        ] {
            if group.is_empty() {
                continue;
            }
            items.push(MenuItem::separator());
            items.push(note(&heading.tr()));
            for network in group {
                let mut title = network.name();
                if network.security.needs_password() && !network.known {
                    title = tr!("{name} (secured)", name = title);
                }
                items.push(
                    item(&title, &Command::Join(network.ssid.clone())).checked(network.current),
                );
            }
        }
    }
    items.push(MenuItem::separator());
    items.push(settings(tr!("Wi-Fi Settings…"), "wifi"));
    Menu::new(items)
}

pub fn sound_menu(sound: &Sound, capturing: &[Capture]) -> Menu {
    let mut items = Vec::new();
    let in_use = in_use(capturing);
    if !in_use.is_empty() {
        items.extend(in_use.iter().map(|line| note(line)));
        items.push(MenuItem::separator());
    }
    items.extend([
        note(&tr!(
            "Volume {percent}%",
            percent = (f64::from(sound.volume) * 100.0).round()
        )),
        item(&tr!("Mute"), &Command::SetMuted(!sound.muted)).checked(sound.muted),
    ]);
    if !sound.outputs.is_empty() {
        items.push(MenuItem::separator());
        items.push(note(&tr!("Output")));
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
    items.push(settings(tr!("Sound Settings…"), "sound"));
    Menu::new(items)
}

pub fn bluetooth_menu(bluetooth: &Bluetooth) -> Menu {
    let mut items = vec![
        item(
            &tr!("Bluetooth"),
            &Command::SetBluetooth(!bluetooth.powered),
        )
        .checked(bluetooth.powered),
    ];
    if bluetooth.powered && !bluetooth.devices.is_empty() {
        items.push(MenuItem::separator());
        items.push(note(&tr!("Devices")));
        for gadget in &bluetooth.devices {
            let title = match gadget.battery {
                Some(percent) => tr!(
                    "{name} · {percent}%",
                    name = gadget.name.clone(),
                    percent = percent
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
            &tr!("Forget Device"),
            Menu::new(forgettable),
        ));
    }
    items.push(MenuItem::separator());
    items.push(settings(tr!("Bluetooth Settings…"), "bluetooth"));
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
    items.push(settings(tr!("Open Keyboard Settings…"), "keyboard"));
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
    Menu::new(vec![
        MenuItem::new(&tr!("About {app}", app = name), "").enabled(false),
        MenuItem::separator(),
        item(&tr!("Hide {app}", app = name), &Command::HideApp),
        item(&tr!("Hide Others"), &Command::HideOthers),
        item(&tr!("Show All"), &Command::ShowAll),
        MenuItem::separator(),
        item(&tr!("Quit {app}", app = name), &Command::QuitApp).shortcut("Ctrl+Q"),
    ])
}

pub fn cascade(menus: &AppMenus<'_>) -> Menu {
    let exported = menus
        .root
        .and_then(|root| root.children.first())
        .filter(|node| !node.children.is_empty());
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
    Menu::new(items).searchable(&tr!("Type to search"), &tr!("No Results"))
}
