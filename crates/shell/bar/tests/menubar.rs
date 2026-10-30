#![allow(clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::time::Duration;

use unbound_bar::menubar::{MenuBar, Slot, app_name};
use unbound_bar::menus::{
    app_menu, battery_menu, bluetooth_menu, input_source_menu, sound_menu, system_menu,
};
use unbound_status::Command;
use unbound_status::dbusmenu::{Node, tree, without_mnemonics};
use unbound_status::devices::battery_of;
use unbound_status::session::{Ending, facts};
use unbound_status::status::{
    Battery, Bluetooth, Gadget, InputSource, InputSources, Output, Power, Sound,
};
use unbound_status::tray::{TrayIcon, pixmap_rgba, split};
use unbound_toolkit::ui::{Menu, MenuItem};
use unbound_toolkit::{Item, Palette, Point, Scheme, TextSystem};
use zbus::zvariant::{OwnedValue, Structure, Value};

fn bar() -> MenuBar {
    MenuBar::new(
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
        "Thu 29 Oct 09:05".to_owned(),
    )
}

fn titles(menu: &Menu) -> Vec<(String, bool, bool)> {
    menu.items
        .iter()
        .filter(|item| !item.separator)
        .map(|item| (item.title.clone(), item.enabled, item.checked))
        .collect()
}

fn action_of<'a>(menu: &'a Menu, title: &str) -> &'a str {
    &menu
        .items
        .iter()
        .find(|item| item.title == title)
        .unwrap_or_else(|| panic!("no {title} in {menu:?}"))
        .action
}

#[test]
fn the_active_app_is_named_by_its_desktop_entry() {
    let names = vec![("org.unbound.Terminal".to_owned(), "Terminal".to_owned())];
    assert_eq!(app_name("org.unbound.Terminal", &names), "Terminal");
    assert_eq!(
        app_name("org.example.Unknown", &names),
        "org.example.Unknown"
    );
}

#[test]
fn the_system_menu_and_app_sit_left_and_status_items_then_the_clock_right() {
    let mut bar = bar();
    bar.set_app("Terminal");
    bar.set_app_menus(vec!["Shell".to_owned(), "Edit".to_owned()]);
    bar.status_mut().sound = Some(Sound {
        volume: 0.5,
        muted: false,
        outputs: Vec::new(),
        default_output: None,
    });
    bar.status_mut().battery = Some(Battery {
        percent: 80.0,
        power: Power::Full,
    });
    let scene = bar.scene(1280, 24, 2.0);
    assert!(
        scene
            .items
            .iter()
            .any(|item| matches!(item, Item::Image { .. })),
        "status symbols are drawn"
    );

    let order: Vec<Slot> = (0..1280)
        .step_by(2)
        .filter_map(|x| bar.slot_at(Point::new(x as f32, 12.0)))
        .fold(Vec::new(), |mut order, slot| {
            if order.last() != Some(&slot) {
                order.push(slot);
            }
            order
        });
    assert_eq!(
        order,
        [
            Slot::System,
            Slot::App,
            Slot::AppMenu(0),
            Slot::AppMenu(1),
            Slot::Sound,
            Slot::Battery,
            Slot::ControlCenter,
            Slot::Clock
        ]
    );
    let tree = bar.accessibility();
    let labels: Vec<String> = tree
        .nodes
        .iter()
        .skip(1)
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect();
    assert!(labels.contains(&"Battery 80%".to_owned()));
    assert!(labels.contains(&"Control Center".to_owned()));
    let (terminal, _) = tree
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Terminal"))
        .expect("the app is announced");
    assert_eq!(bar.slot_of_node(*terminal), Some(Slot::App));
    let clock = bar.slot_rect(Slot::Clock).expect("clock");
    assert!(clock.x + clock.width <= 1280.0 && clock.x + clock.width > 1260.0);
}

#[test]
fn an_open_menu_highlights_its_item() {
    let mut bar = bar();
    bar.scene(800, 24, 1.0);
    let plain = bar.scene(800, 24, 1.0).items.len();
    assert!(bar.set_open(Some(Slot::System)));
    assert_eq!(bar.scene(800, 24, 1.0).items.len(), plain + 1);
    assert!(!bar.set_open(Some(Slot::System)));
}

#[test]
fn every_command_survives_its_trip_through_a_menu_action() {
    for command in [
        Command::About,
        Command::Settings(None),
        Command::Settings(Some("wifi".to_owned())),
        Command::ForceQuit,
        Command::Sleep,
        Command::Restart,
        Command::ShutDown,
        Command::LockScreen,
        Command::LogOut,
        Command::SetWifi(false),
        Command::Join(b"caf\xc3\xa9:5G".to_vec()),
        Command::SetMuted(true),
        Command::UseOutput("alsa_output.usb".to_owned()),
        Command::SetBluetooth(true),
        Command::ToggleGadget("AA:BB:CC:DD:EE:FF".to_owned()),
        Command::Item {
            item: ":1.42/MenuBar".to_owned(),
            id: 7,
        },
        Command::AboutApp("1.2.3".to_owned()),
        Command::HideApp,
        Command::HideOthers,
        Command::ShowAll,
        Command::SelectInputSource(2),
    ] {
        assert_eq!(Command::parse(&command.action()), Some(command));
    }
    assert_eq!(Command::parse("unknown"), None);
}

#[test]
fn the_system_menu_offers_power_and_session_commands_with_the_users_name() {
    let menu = system_menu("Ada Lovelace");
    let shown = titles(&menu);
    assert_eq!(
        shown.first().map(|item| item.0.as_str()),
        Some("About This Computer")
    );
    assert_eq!(
        shown.last().map(|item| item.0.as_str()),
        Some("Log Out Ada Lovelace…")
    );
    assert_eq!(
        Command::parse(action_of(&menu, "Force Quit…")),
        Some(Command::ForceQuit)
    );
    assert_eq!(
        Command::parse(action_of(&menu, "Shut Down…")),
        Some(Command::ShutDown)
    );
}

#[test]
fn the_battery_menu_tells_the_time_left_and_the_power_source() {
    let on_battery = Battery {
        percent: 64.4,
        power: Power::Battery {
            remaining: Some(Duration::from_secs(3 * 3600 + 25 * 60)),
        },
    };
    let shown: Vec<String> = titles(&battery_menu(&on_battery))
        .into_iter()
        .map(|item| item.0)
        .collect();
    assert_eq!(
        shown,
        ["Battery 64%", "3:25 remaining", "Power Source: Battery"]
    );

    let charging = battery_of(1, 50.0, 0, 1800);
    assert_eq!(
        charging.power,
        Power::Charging {
            until_full: Some(Duration::from_secs(1800))
        }
    );
    assert_eq!(titles(&battery_menu(&charging))[1].0, "0:30 until full");
    assert_eq!(battery_of(4, 100.0, 0, 0).power, Power::Full);
    assert_eq!(
        battery_of(2, 30.0, 0, 0).power,
        Power::Battery { remaining: None }
    );
}

#[test]
fn the_sound_menu_checks_the_output_in_use_and_mute_switches() {
    let sound = Sound {
        volume: 0.42,
        muted: false,
        outputs: vec![
            Output {
                name: "speakers".to_owned(),
                description: "Speakers".to_owned(),
            },
            Output {
                name: "usb".to_owned(),
                description: "USB Headset".to_owned(),
            },
        ],
        default_output: Some("usb".to_owned()),
    };
    let menu = sound_menu(&sound);
    let shown = titles(&menu);
    assert!(shown.contains(&("Volume 42%".to_owned(), false, false)));
    assert!(shown.contains(&("USB Headset".to_owned(), true, true)));
    assert!(shown.contains(&("Speakers".to_owned(), true, false)));
    assert_eq!(
        Command::parse(action_of(&menu, "Mute")),
        Some(Command::SetMuted(true))
    );
    assert_eq!(
        Command::parse(action_of(&menu, "Speakers")),
        Some(Command::UseOutput("speakers".to_owned()))
    );
}

#[test]
fn the_bluetooth_menu_lists_devices_only_while_bluetooth_is_on() {
    let headphones = Gadget {
        address: "AA:BB".to_owned(),
        name: "Headphones".to_owned(),
        connected: true,
    };
    let on = Bluetooth {
        powered: true,
        devices: vec![headphones.clone()],
    };
    let menu = bluetooth_menu(&on);
    assert!(titles(&menu).contains(&("Headphones".to_owned(), true, true)));
    assert_eq!(
        Command::parse(action_of(&menu, "Bluetooth")),
        Some(Command::SetBluetooth(false))
    );

    let off = Bluetooth {
        powered: false,
        devices: vec![headphones],
    };
    assert!(
        !titles(&bluetooth_menu(&off))
            .iter()
            .any(|item| item.0 == "Headphones")
    );
}

fn layout_node(
    id: i32,
    properties: &[(&str, Value<'static>)],
    children: Vec<Value<'static>>,
) -> Value<'static> {
    let properties: HashMap<String, Value<'static>> = properties
        .iter()
        .map(|(name, value)| ((*name).to_owned(), Value::Value(Box::new(value.clone()))))
        .collect();
    Value::Structure(Structure::from((id, properties, children)))
}

#[test]
fn a_dbusmenu_layout_becomes_a_menu_without_mnemonics_or_hidden_items() {
    assert_eq!(without_mnemonics("_Open File"), "Open File");
    assert_eq!(without_mnemonics("Save__As"), "Save_As");

    let children = vec![
        Value::Value(Box::new(layout_node(
            1,
            &[("label", Value::from("_Show"))],
            Vec::new(),
        ))),
        Value::Value(Box::new(layout_node(
            2,
            &[("type", Value::from("separator"))],
            Vec::new(),
        ))),
        Value::Value(Box::new(layout_node(
            3,
            &[
                ("label", Value::from("Hidden")),
                ("visible", Value::from(false)),
            ],
            Vec::new(),
        ))),
        Value::Value(Box::new(layout_node(
            4,
            &[
                ("label", Value::from("Mute")),
                ("toggle-type", Value::from("checkmark")),
                ("toggle-state", Value::from(1)),
                ("enabled", Value::from(false)),
            ],
            Vec::new(),
        ))),
        Value::Value(Box::new(layout_node(
            5,
            &[("label", Value::from("More"))],
            vec![Value::Value(Box::new(layout_node(
                6,
                &[("label", Value::from("Quit"))],
                Vec::new(),
            )))],
        ))),
    ];
    let root: (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>) = (
        0,
        HashMap::new(),
        children
            .into_iter()
            .map(|child| OwnedValue::try_from(child).expect("owned"))
            .collect(),
    );
    let node: Node = tree(root);
    let converted = app_menu(&node, ":1.9/MenuBar");

    assert_eq!(converted.items.len(), 4);
    assert_eq!(
        converted.items[0],
        MenuItem::new(
            "Show",
            &Command::Item {
                item: ":1.9/MenuBar".to_owned(),
                id: 1
            }
            .action()
        )
    );
    assert!(converted.items[1].separator);
    assert_eq!(
        (
            converted.items[2].title.as_str(),
            converted.items[2].checked,
            converted.items[2].enabled
        ),
        ("Mute", true, false)
    );
    let more = converted.items[3].submenu.as_ref().expect("a submenu");
    assert_eq!(more.items[0].title, "Quit");
}

#[test]
fn an_app_menu_shows_shortcuts_and_leaves_about_and_hiding_to_the_menu_bar() {
    let entries = [
        layout_node(
            1,
            &[
                ("label", Value::from("About Files")),
                ("x-unbound-shell-action", Value::from("about")),
                ("x-unbound-version", Value::from("0.4.0")),
            ],
            Vec::new(),
        ),
        layout_node(
            2,
            &[
                ("label", Value::from("Hide Files")),
                ("shortcut", Value::from(vec![vec!["Super", "H"]])),
                ("x-unbound-shell-action", Value::from("hide")),
            ],
            Vec::new(),
        ),
        layout_node(
            3,
            &[
                ("label", Value::from("New Folder")),
                ("shortcut", Value::from(vec![vec!["Shift", "Super", "N"]])),
            ],
            Vec::new(),
        ),
    ];
    let root = (
        0,
        HashMap::new(),
        entries
            .into_iter()
            .map(|entry| OwnedValue::try_from(Value::Value(Box::new(entry))).expect("owned"))
            .collect(),
    );
    let converted = app_menu(&tree(root), ":1.9/org/unbound/MenuBar");

    let actions: Vec<_> = converted
        .items
        .iter()
        .map(|item| Command::parse(&item.action))
        .collect();
    assert_eq!(
        actions,
        [
            Some(Command::AboutApp("0.4.0".to_owned())),
            Some(Command::HideApp),
            Some(Command::Item {
                item: ":1.9/org/unbound/MenuBar".to_owned(),
                id: 3
            }),
        ]
    );
    assert_eq!(converted.items[0].shortcut, None);
    assert_eq!(converted.items[1].shortcut.as_deref(), Some("Super+H"));
    assert_eq!(
        converted.items[2].shortcut.as_deref(),
        Some("Shift+Super+N")
    );
}

#[test]
fn status_item_ids_and_pixmaps_are_understood() {
    assert_eq!(
        split(":1.5/StatusNotifierItem"),
        (":1.5", "/StatusNotifierItem")
    );
    assert_eq!(
        split("org.example.Tray"),
        ("org.example.Tray", "/StatusNotifierItem")
    );
    assert_eq!(
        pixmap_rgba(1, 1, &[0xFF, 0x10, 0x20, 0x30]),
        Some(TrayIcon::Pixels {
            width: 1,
            height: 1,
            rgba: vec![0x10, 0x20, 0x30, 0xFF]
        })
    );
    assert_eq!(pixmap_rgba(2, 2, &[0; 4]), None);
}

#[test]
fn about_this_computer_names_the_system_chip_and_memory() {
    let found = facts(
        "studio\n",
        "NAME=\"Debian GNU/Linux\"\nPRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\n",
        Some("Apple Mac Studio (M2 Max, 2023)\0"),
        "",
        "MemTotal:       32768000 kB\nMemFree: 1 kB\n",
    );
    assert_eq!(found.name, "studio");
    assert_eq!(found.system, "Debian GNU/Linux 13 (trixie)");
    assert_eq!(found.chip, "Apple Mac Studio (M2 Max, 2023)");
    assert_eq!(found.memory, "31 GB");
    assert_eq!(
        Ending::from_keyword(Ending::ShutDown.keyword()),
        Some(Ending::ShutDown)
    );
}

fn sources(active: usize, shorts: &[(&str, &str)]) -> InputSources {
    InputSources {
        sources: shorts
            .iter()
            .map(|(name, short)| InputSource {
                name: (*name).to_owned(),
                short: (*short).to_owned(),
            })
            .collect(),
        active,
    }
}

#[test]
fn the_input_source_item_appears_only_with_more_than_one_source() {
    let mut bar = bar();
    bar.status_mut().input_sources = sources(0, &[("Swedish", "SV")]);
    bar.scene(1280, 24, 1.0);
    assert_eq!(bar.slot_rect(Slot::InputSource), None);

    bar.status_mut().input_sources = sources(1, &[("Swedish", "SV"), ("English (US)", "EN")]);
    bar.scene(1280, 24, 1.0);
    let badge = bar.slot_rect(Slot::InputSource).expect("shown");
    let control_center = bar.slot_rect(Slot::ControlCenter).expect("control center");
    assert!(badge.x + badge.width <= control_center.x);
    let labels: Vec<String> = bar
        .accessibility()
        .nodes
        .iter()
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect();
    assert!(labels.contains(&"Input source: English (US)".to_owned()));
}

#[test]
fn the_input_source_menu_checks_the_active_source_and_opens_keyboard_settings() {
    let menu = input_source_menu(&sources(1, &[("Swedish", "SV"), ("German", "DE")]));
    assert_eq!(
        titles(&menu),
        [
            ("Swedish".to_owned(), true, false),
            ("German".to_owned(), true, true),
            ("Open Keyboard Settings…".to_owned(), true, false),
        ]
    );
    assert_eq!(
        Command::parse(action_of(&menu, "Swedish")),
        Some(Command::SelectInputSource(0))
    );
    assert_eq!(
        Command::parse(action_of(&menu, "Open Keyboard Settings…")),
        Some(Command::Settings(Some("keyboard".to_owned())))
    );
}
