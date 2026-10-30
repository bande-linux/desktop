#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId};
use unbound_settings::{
    About, AboutCommand, Category, Command, PaneId, SettingId, SettingsView, Storage,
    StorageCommand, Update, Value, categorize, chip_name, memory_bytes, static_hostname,
    system_version, tally,
};
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{Palette, Scheme, TextSystem};

const SIZE: (u32, u32) = (900, 600);

fn nodes(view: &mut SettingsView) -> Vec<(NodeId, Node)> {
    view.scene(SIZE.0, SIZE.1);
    view.accessibility().nodes
}

fn find(view: &mut SettingsView, role: Role, label: &str) -> Option<NodeId> {
    nodes(view)
        .into_iter()
        .find(|(_, node)| {
            node.role() == role && node.label().is_some_and(|text| text.starts_with(label))
        })
        .map(|(id, _)| id)
}

fn has_text(view: &mut SettingsView, text: &str) -> bool {
    nodes(view).into_iter().any(|(_, node)| {
        node.label().is_some_and(|label| label.contains(text))
            || node.value().is_some_and(|value| value.contains(text))
    })
}

fn click(view: &mut SettingsView, target: NodeId) {
    view.perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: target,
        data: None,
    });
}

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

pub fn general_view(scheme: Scheme) -> SettingsView {
    let mut view = SettingsView::new(Palette::for_scheme(scheme), TextSystem::new());
    view.apply(Update::About(about()));
    view
}

#[test]
fn general_lists_its_pages_and_back_returns_to_the_page_you_came_from() {
    let mut view = general_view(Scheme::Light);
    assert!(view.visible_panes().contains(&PaneId::General));
    assert!(!view.visible_panes().contains(&PaneId::About));
    view.select_pane(PaneId::General);
    let about = find(&mut view, Role::Button, "About").expect("About row");
    assert!(has_text(&mut view, "About, Apple MacBook Pro"));
    assert!(has_text(&mut view, "Storage, 210 GB available of 494 GB"));
    click(&mut view, about);
    assert_eq!(view.selected_pane(), PaneId::About);
    assert!(has_text(&mut view, "Apple M1 Pro"));
    assert!(has_text(&mut view, "C02XL0GHJGH5"));
    let selected: Vec<String> = nodes(&mut view)
        .into_iter()
        .filter(|(_, node)| node.role() == Role::ListItem && node.is_selected() == Some(true))
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect();
    assert_eq!(selected, ["General"]);

    key(
        &mut view,
        Key::Character('['),
        Modifiers {
            logo: true,
            ..Modifiers::default()
        },
    );
    assert_eq!(view.selected_pane(), PaneId::General);
    assert_eq!(view.focused_item().as_deref(), Some("about"));

    let storage = find(&mut view, Role::Button, "Storage").expect("Storage row");
    click(&mut view, storage);
    let back = find(&mut view, Role::Button, "‹ General").expect("back button");
    click(&mut view, back);
    assert_eq!(view.selected_pane(), PaneId::General);
}

#[test]
fn renaming_the_computer_waits_for_return_and_escape_keeps_the_old_name() {
    let mut view = general_view(Scheme::Light);
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

pub fn storage_view(scheme: Scheme) -> SettingsView {
    let mut view = general_view(scheme);
    view.select_pane(PaneId::Storage);
    view.apply(Update::Storage(storage()));
    view
}

#[test]
fn storage_measures_when_opened_and_asks_before_emptying_the_trash() {
    let mut view = general_view(Scheme::Light);
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
