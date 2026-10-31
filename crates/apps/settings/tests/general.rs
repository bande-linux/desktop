#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::{Path, PathBuf};

use accesskit::{Action, ActionRequest, Role, TreeId};
use support::{SIZE, click, find, has_text, nodes};
use unbound_settings::{
    About, AboutCommand, App, Category, Command, DefaultAppCommand, InstalledLocale, LocaleState,
    LoginCommand, LoginItems, NetworkCommand, Package, PaneId, SettingId, SettingsView,
    SoftwareState, Storage, StorageCommand, Update, Value, WifiState, assignments, categorize,
    chip_name, installed_locales, locale_name, login_items, login_matches, memory_bytes,
    parse_assignments, static_hostname, system_version, tally,
};
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{Item, Palette, Scheme, TextSystem};
use unbound_xdg::DesktopEntry;

fn key(view: &mut SettingsView, key: Key, modifiers: Modifiers) {
    view.handle(&Event::Key { key, modifiers });
    view.scene(SIZE.0, SIZE.1);
}

fn about() -> About {
    About {
        name: "Studio".to_owned(),
        model: "Apple MacBook Pro (14-inch, M1 Pro, 2021)".to_owned(),
        chip: "Apple M1 Pro".to_owned(),
        memory: 16_000_000_000,
        capacity: 494_000_000_000,
        available: 210_000_000_000,
        unbound: "0.1.0".to_owned(),
        system: "Debian 13.7 (trixie)".to_owned(),
        serial: Some("C02XL0GHJGH5".to_owned()),
    }
}

fn general_view() -> SettingsView {
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.apply(Update::About(about()));
    view
}

fn selected_in_sidebar(view: &mut SettingsView) -> Vec<String> {
    nodes(view)
        .into_iter()
        .filter(|(_, node)| node.role() == Role::ListItem && node.is_selected() == Some(true))
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect()
}

#[test]
fn about_opens_storage_and_software_update_and_back_returns_to_the_row_you_came_from() {
    let mut view = general_view();
    assert!(view.visible_panes().contains(&PaneId::About));
    assert!(!view.visible_panes().contains(&PaneId::Storage));
    view.select_pane(PaneId::About);
    assert!(has_text(&mut view, "Studio"));
    assert!(has_text(
        &mut view,
        "Apple MacBook Pro (14-inch, M1 Pro, 2021)"
    ));
    assert!(has_text(&mut view, "C02XL0GHJGH5"));
    assert!(has_text(&mut view, "210 GB available of 494 GB"));
    let storage = find(&mut view, Role::Button, "Storage").expect("Storage row");
    click(&mut view, storage);
    assert_eq!(view.selected_pane(), PaneId::Storage);
    assert_eq!(selected_in_sidebar(&mut view), ["About"]);

    key(
        &mut view,
        Key::Character('['),
        Modifiers {
            logo: true,
            ..Modifiers::default()
        },
    );
    assert_eq!(view.selected_pane(), PaneId::About);
    assert_eq!(view.focused_item().as_deref(), Some("storage"));

    let update = find(&mut view, Role::Button, "Software Update").expect("update row");
    click(&mut view, update);
    assert_eq!(view.selected_pane(), PaneId::SoftwareUpdate);
    let back = find(&mut view, Role::Button, "‹ About").expect("back button");
    click(&mut view, back);
    assert_eq!(view.selected_pane(), PaneId::About);
    assert_eq!(view.focused_item().as_deref(), Some("software-update"));
}

#[test]
fn the_sidebar_offers_waiting_updates_only_while_they_wait() {
    let mut view = general_view();
    assert!(find(&mut view, Role::ListItem, "2 updates are available").is_none());
    view.apply(Update::Software(SoftwareState::Available(vec![
        Package::from_id("libc6;2.41-12;arm64;debian", ""),
        Package::from_id("firefox-esr;128.4.0esr-1;arm64;debian", ""),
    ])));
    let waiting = find(&mut view, Role::ListItem, "2 updates are available").expect("row");
    click(&mut view, waiting);
    assert_eq!(view.selected_pane(), PaneId::SoftwareUpdate);

    view.apply(Update::Software(SoftwareState::Installed { restart: true }));
    assert!(find(&mut view, Role::ListItem, "Restart to finish updating").is_some());
    view.apply(Update::Software(SoftwareState::UpToDate));
    assert!(find(&mut view, Role::ListItem, "Restart to finish updating").is_none());
    assert!(find(&mut view, Role::ListItem, "2 updates").is_none());
}

#[test]
fn a_link_to_a_pane_without_its_device_opens_a_calm_page_and_stays_there() {
    let mut view = general_view();
    view.select_pane(PaneId::Bluetooth);
    assert!(!view.visible_panes().contains(&PaneId::Bluetooth));
    assert!(has_text(&mut view, "Bluetooth isn't available"));
    assert!(find(&mut view, Role::Switch, "Bluetooth").is_none());
    view.apply(Update::Values(vec![(
        SettingId::Clock24Hour,
        Value::Flag(true),
    )]));
    assert_eq!(view.selected_pane(), PaneId::Bluetooth);
}

#[test]
fn the_clipboard_history_is_cleared_only_after_confirming_and_escape_keeps_it() {
    let mut view = general_view();
    view.set_value(SettingId::LockAfter, Value::Seconds(600));
    view.select_pane(PaneId::PrivacySecurity);
    assert!(has_text(&mut view, "After 10 min"));

    let ask = find(&mut view, Role::Button, "Clear History…").expect("clear button");
    click(&mut view, ask);
    assert!(has_text(&mut view, "Clear the clipboard history?"));
    key(&mut view, Key::Escape, Modifiers::default());
    assert!(!has_text(&mut view, "Clear the clipboard history?"));
    assert!(view.take_commands().is_empty());

    let ask = find(&mut view, Role::Button, "Clear History…").expect("clear button");
    click(&mut view, ask);
    let clear = find(&mut view, Role::Button, "Clear").expect("confirm");
    click(&mut view, clear);
    assert_eq!(view.take_commands(), [Command::ClearClipboard]);
    assert!(!has_text(&mut view, "Clear the clipboard history?"));

    let lock = find(&mut view, Role::Button, "Automatic lock").expect("lock row");
    click(&mut view, lock);
    assert_eq!(view.selected_pane(), PaneId::Power);
}

#[test]
fn the_sidebar_groups_its_panes_under_four_labels() {
    let mut view = general_view();
    let headings: Vec<String> = nodes(&mut view)
        .into_iter()
        .filter(|(_, node)| node.role() == Role::Heading)
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect();
    assert_eq!(
        headings[..4],
        ["Desktop", "Devices", "System", "Appearance"].map(str::to_owned)[..],
        "without network, sound or displays the Connections group is left out"
    );
    assert_eq!(
        view.visible_panes(),
        [
            PaneId::Appearance,
            PaneId::Bar,
            PaneId::Accessibility,
            PaneId::Keyboard,
            PaneId::Power,
            PaneId::PrivacySecurity,
            PaneId::Apps,
            PaneId::About,
        ]
    );
}

#[test]
fn renaming_the_computer_waits_for_return_and_escape_keeps_the_old_name() {
    let mut view = general_view();
    view.select_pane(PaneId::About);
    view.take_commands();
    let field = nodes(&mut view)
        .into_iter()
        .find(|(_, node)| node.role() == Role::TextInput && node.value() == Some("Studio"))
        .map(|(id, _)| id)
        .expect("name field");
    view.perform(&ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node: field,
        data: None,
    });
    key(&mut view, Key::End, Modifiers::default());
    for character in " Two".chars() {
        key(&mut view, Key::Character(character), Modifiers::default());
    }
    assert!(
        view.take_commands().is_empty(),
        "nothing is written while typing"
    );
    key(&mut view, Key::Enter, Modifiers::default());
    assert_eq!(
        view.take_commands(),
        [Command::About(AboutCommand::Rename(
            "Studio Two".to_owned()
        ))]
    );
    assert_eq!(
        view.value(SettingId::ComputerName),
        Some(&Value::Text("Studio Two".to_owned()))
    );

    for character in "xyz".chars() {
        key(&mut view, Key::Character(character), Modifiers::default());
    }
    key(&mut view, Key::Escape, Modifiers::default());
    assert!(view.take_commands().is_empty());
    assert!(has_text(&mut view, "Studio Two"));
    assert!(!has_text(&mut view, "Studio Twoxyz"));
}

fn home() -> PathBuf {
    std::env::home_dir().expect("home")
}

fn storage() -> Storage {
    Storage {
        capacity: 494_000_000_000,
        available: 210_000_000_000,
        categories: Some(vec![
            (Category::Apps, 21_000_000_000),
            (Category::Documents, 182_000_000_000),
            (Category::Trash, 4_100_000_000),
            (Category::System, 76_900_000_000),
        ]),
        largest: vec![
            (home().join("Movies/Holiday.mov"), 9_800_000_000),
            (home().join("Downloads/debian.iso"), 3_900_000_000),
        ],
        trash_items: 12,
    }
}

#[test]
fn storage_measures_when_opened_and_asks_before_emptying_the_trash() {
    let mut view = general_view();
    view.select_pane(PaneId::Storage);
    assert_eq!(
        view.take_commands(),
        [Command::Storage(StorageCommand::Measure)]
    );
    assert!(has_text(&mut view, "Calculating…"));
    view.apply(Update::Storage(storage()));
    assert!(has_text(&mut view, "Documents 182 GB"));
    assert!(has_text(&mut view, "Holiday.mov"));

    let empty = find(&mut view, Role::Button, "Empty Trash…").expect("Empty Trash…");
    click(&mut view, empty);
    assert!(view.take_commands().is_empty());
    assert!(has_text(&mut view, "Erase all 12 items in the Trash?"));
    key(&mut view, Key::Escape, Modifiers::default());
    assert!(!has_text(&mut view, "Erase all"));

    let empty = find(&mut view, Role::Button, "Empty Trash…").expect("Empty Trash…");
    click(&mut view, empty);
    let confirm = find(&mut view, Role::Button, "Empty Trash").expect("confirm");
    click(&mut view, confirm);
    assert_eq!(
        view.take_commands(),
        [Command::Storage(StorageCommand::EmptyTrash)]
    );

    assert!(has_text(&mut view, "Holiday.mov, ~/Movies"));
    let show = find(&mut view, Role::Button, "Show in Files").expect("Show in Files");
    click(&mut view, show);
    assert_eq!(
        view.take_commands(),
        [Command::Reveal(home().join("Movies/Holiday.mov"))]
    );
}

#[test]
fn system_facts_read_as_people_know_them() {
    assert_eq!(
        chip_name("apple,j314s\0apple,t6000\0apple,arm-platform\0", ""),
        "Apple M1 Pro"
    );
    let cpuinfo =
        "processor\t: 0\nCPU implementer\t: 0x61\n\nprocessor\t: 1\nCPU implementer\t: 0x61\n";
    assert_eq!(chip_name("", cpuinfo), "Apple silicon (2 cores)");
    assert_eq!(
        memory_bytes("MemTotal:       16151040 kB\n"),
        16_000_000_000
    );
    assert_eq!(
        system_version(
            "PRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\nID=debian\nVERSION_CODENAME=trixie\n",
            Some("13.7\n")
        ),
        "Debian 13.7 (trixie)"
    );
    assert_eq!(
        system_version(
            "PRETTY_NAME=\"Debian GNU/Linux trixie/sid\"\nID=debian\n",
            Some("trixie/sid")
        ),
        "Debian GNU/Linux trixie/sid"
    );
    assert_eq!(static_hostname("Ada’s MacBook Pro"), "adas-macbook-pro");
    assert_eq!(static_hostname("  --Studio__2--  "), "studio-2");
    assert_eq!(static_hostname("Café"), "caf");
}

#[test]
fn storage_splits_used_space_and_finds_the_largest_files() {
    assert_eq!(
        categorize(100, 20, 50, 0),
        [
            (Category::Apps, 20),
            (Category::Documents, 50),
            (Category::System, 30)
        ]
    );
    let home = tempfile::tempdir().expect("home");
    let write = |path: &str, bytes: usize| {
        let path = home.path().join(path);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("folder");
        std::fs::write(path, vec![7u8; bytes]).expect("file");
    };
    write("Movies/big.mov", 300_000);
    write("Documents/notes.txt", 10_000);
    write("Downloads/medium.iso", 200_000);
    write(".local/share/Trash/files/old.bin", 400_000);
    let trash = home.path().join(".local/share/Trash");
    let tally = tally(home.path(), &[trash.as_path()]);
    let names: Vec<&Path> = tally
        .largest
        .iter()
        .map(|(path, _)| path.strip_prefix(home.path()).expect("inside home"))
        .collect();
    assert_eq!(
        names,
        [
            Path::new("Movies/big.mov"),
            Path::new("Downloads/medium.iso"),
            Path::new("Documents/notes.txt")
        ]
    );
    assert!(tally.bytes >= 510_000);
    assert!(tally.bytes < 900_000);
}

fn app(id: &str, name: &str) -> App {
    App {
        id: id.to_owned(),
        name: name.to_owned(),
    }
}

#[test]
fn login_items_are_added_by_name_and_removed_from_their_row() {
    let mut view = general_view();
    view.select_pane(PaneId::Apps);
    assert_eq!(
        view.take_commands(),
        [
            Command::Login(LoginCommand::Refresh),
            Command::DefaultApp(DefaultAppCommand::Refresh)
        ]
    );
    view.apply(Update::Login(LoginItems {
        items: vec![app("org.unbound.Terminal.desktop", "Terminal")],
        apps: vec![
            app("org.mozilla.firefox.desktop", "Firefox"),
            app("org.unbound.Files.desktop", "Files"),
            app("org.unbound.Terminal.desktop", "Terminal"),
        ],
    }));
    let add = find(&mut view, Role::Button, "Add App…").expect("Add App…");
    click(&mut view, add);
    for character in "fi".chars() {
        key(&mut view, Key::Character(character), Modifiers::default());
    }
    assert!(has_text(&mut view, "Firefox"));
    assert!(has_text(&mut view, "Files"));
    key(&mut view, Key::Enter, Modifiers::default());
    assert_eq!(
        view.take_commands(),
        [Command::Login(LoginCommand::Add(
            "org.mozilla.firefox.desktop".to_owned()
        ))]
    );
    let remove = find(&mut view, Role::Button, "Remove").expect("Remove");
    click(&mut view, remove);
    assert_eq!(
        view.take_commands(),
        [Command::Login(LoginCommand::Remove(
            "org.unbound.Terminal.desktop".to_owned()
        ))]
    );
}

#[test]
fn login_items_list_what_opens_at_login_and_search_skips_them() {
    let home = tempfile::tempdir().expect("home");
    let dir = home.path().join("autostart");
    std::fs::create_dir_all(&dir).expect("autostart");
    let write = |id: &str, text: &str| std::fs::write(dir.join(id), text).expect("entry");
    write(
        "org.example.Mail.desktop",
        "[Desktop Entry]\nType=Application\nName=Mail\nExec=mail\n",
    );
    write(
        "org.example.Old.desktop",
        "[Desktop Entry]\nType=Application\nName=Old\nExec=old\nHidden=true\n",
    );
    write("notes.txt", "not an entry");
    let source = "[Desktop Entry]\nType=Application\nName=Chat\nExec=chat\n";
    let chat = DesktopEntry::parse(
        "org.example.Chat.desktop",
        &dir.join("x"),
        source,
        None,
        &[],
    )
    .expect("entry");
    write("org.example.Chat.desktop", source);
    let items = login_items(&dir, &[chat]);
    assert_eq!(
        items,
        [
            app("org.example.Chat.desktop", "Chat"),
            app("org.example.Mail.desktop", "Mail")
        ]
    );
    let state = LoginItems {
        items,
        apps: vec![
            app("org.example.Chat.desktop", "Chat"),
            app("org.example.Calc.desktop", "Calculator"),
            app("org.example.Cam.desktop", "Web Camera"),
        ],
    };
    assert_eq!(
        login_matches(&state, "ca", 8),
        ["org.example.Calc.desktop", "org.example.Cam.desktop"]
    );
    assert!(login_matches(&state, "chat", 8).is_empty());
}

fn network_state() -> unbound_settings::NetworkState {
    unbound_settings::NetworkState {
        ethernet: Some(unbound_network::Ethernet {
            interface: "end0".to_owned(),
            cable: true,
            managed: true,
            connected: true,
            address: Some("192.168.1.20/24".to_owned()),
            router: Some("192.168.1.1".to_owned()),
            dns: vec!["192.168.1.1".to_owned()],
            manual: false,
        }),
        vpns: vec![unbound_network::Vpn {
            path: "/org/freedesktop/NetworkManager/Settings/7".to_owned(),
            name: "Office".to_owned(),
            state: unbound_network::VpnState::Off,
        }],
    }
}

#[test]
fn ethernet_switches_to_a_manual_address_and_vpns_connect_from_their_row() {
    let mut view = general_view();
    view.apply(Update::Network(network_state()));
    view.select_pane(PaneId::Network);
    assert!(has_text(
        &mut view,
        "Connected as 192.168.1.20/24, Router 192.168.1.1 · DNS 192.168.1.1"
    ));
    let connect = find(&mut view, Role::Button, "Connect").expect("Connect");
    click(&mut view, connect);
    assert_eq!(
        view.take_commands(),
        [Command::Network(NetworkCommand::ConnectVpn(
            "/org/freedesktop/NetworkManager/Settings/7".to_owned()
        ))]
    );
    let manual = find(&mut view, Role::RadioButton, "Manually").expect("Manually");
    click(&mut view, manual);
    assert!(
        view.take_commands().is_empty(),
        "nothing changes until an address is typed"
    );
    let router = nodes(&mut view)
        .into_iter()
        .find(|(_, node)| node.role() == Role::TextInput && node.value() == Some("192.168.1.1"))
        .map(|(id, _)| id)
        .expect("router field");
    view.perform(&ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node: router,
        data: None,
    });
    key(&mut view, Key::End, Modifiers::default());
    key(&mut view, Key::Backspace, Modifiers::default());
    key(&mut view, Key::Character('2'), Modifiers::default());
    key(&mut view, Key::Enter, Modifiers::default());
    let commands = view.take_commands();
    let [Command::Network(NetworkCommand::ConfigureEthernet(Some(manual)))] = commands.as_slice()
    else {
        panic!("unexpected {commands:?}");
    };
    assert_eq!(manual.router, Some(std::net::Ipv4Addr::new(192, 168, 1, 2)));
    assert_eq!(manual.prefix, 24);
}

#[test]
fn languages_come_from_installed_locales_and_write_lang_language_and_formats() {
    let sources = tempfile::tempdir().expect("sources");
    std::fs::write(
        sources.path().join("sv_SE"),
        "title      \"Swedish locale for Sweden\"\nlanguage   \"Swedish\"\nterritory  \"Sweden\"\n",
    )
    .expect("source");
    let installed = installed_locales(
        "C\nC.utf8\nPOSIX\nsv_SE.utf8\nen_GB.utf8\nde_DE\n",
        sources.path(),
    );
    assert_eq!(
        installed,
        [
            InstalledLocale {
                code: "en_GB.UTF-8".to_owned(),
                name: "en_GB".to_owned()
            },
            InstalledLocale {
                code: "sv_SE.UTF-8".to_owned(),
                name: "Swedish (Sweden)".to_owned()
            },
        ]
    );
    assert_eq!(locale_name("xx_YY", ""), "xx_YY");
    assert_eq!(
        assignments("sv_SE.UTF-8", Some("en_GB.UTF-8")),
        [
            "LANG=sv_SE.UTF-8",
            "LANGUAGE=sv_SE:sv",
            "LC_TIME=en_GB.UTF-8",
            "LC_NUMERIC=en_GB.UTF-8",
            "LC_MONETARY=en_GB.UTF-8",
            "LC_MEASUREMENT=en_GB.UTF-8",
            "LC_PAPER=en_GB.UTF-8"
        ]
    );
    assert_eq!(assignments("sv_SE.UTF-8", Some("sv_SE.UTF-8")).len(), 2);
    assert_eq!(
        parse_assignments(&[
            "LANG=en_GB.UTF-8".to_owned(),
            "LC_TIME=sv_SE.UTF-8".to_owned()
        ]),
        ("en_GB.UTF-8".to_owned(), Some("sv_SE.UTF-8".to_owned()))
    );

    let mut view = general_view();
    view.apply(Update::Locale(Some(LocaleState {
        installed,
        language: "en_GB.UTF-8".to_owned(),
        region: None,
    })));
    view.select_pane(PaneId::LanguageTime);
    assert!(has_text(
        &mut view,
        "Changes take effect the next time you log in"
    ));
    let region = find(&mut view, Role::ComboBox, "Region").expect("Region menu");
    view.perform(&ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node: region,
        data: None,
    });
    assert_eq!(view.focused_setting(), Some(SettingId::Region));
    assert!(has_text(&mut view, "Sat 14 Mar, 13:45"));
}

fn crossfade(items: &[Item]) -> Vec<f32> {
    items
        .iter()
        .flat_map(|item| match item {
            Item::Opacity { alpha, items } if !items.is_empty() => vec![*alpha],
            Item::Clip { items, .. } | Item::Translate { items, .. } => crossfade(items),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn turning_wifi_off_cross_fades_the_page_into_the_off_state_but_changing_panes_does_not() {
    let mut view = general_view();
    view.apply(Update::Wifi(Some(WifiState {
        enabled: true,
        networks: Vec::new(),
    })));
    view.select_pane(PaneId::Wifi);
    assert!(crossfade(&view.scene(SIZE.0, SIZE.1).items).is_empty());

    view.apply(Update::Wifi(Some(WifiState {
        enabled: false,
        networks: Vec::new(),
    })));
    let alphas = crossfade(&view.scene(SIZE.0, SIZE.1).items);
    assert_eq!(
        alphas.len(),
        2,
        "the old page fades out while the off state fades in"
    );
    assert!((alphas[0] + alphas[1] - 1.0).abs() < 1e-4);
    assert!(has_text(&mut view, "Wi-Fi is off"));

    view.select_pane(PaneId::About);
    assert!(crossfade(&view.scene(SIZE.0, SIZE.1).items).is_empty());
}
