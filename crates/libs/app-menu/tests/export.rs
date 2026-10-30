#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::time::{Duration, Instant};

use unbound_app_menu::{AppMenu, Entry, MENU_PATH};
use unbound_test_bus::SessionBus;
use zbus::blocking::Connection;
use zbus::blocking::fdo::DBusProxy;
use zbus::zvariant::{OwnedValue, Value};

const APP: &str = "org.unbound.Example";

type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

fn menus() -> Vec<Entry> {
    vec![
        Entry::submenu(
            "Example",
            vec![
                Entry::item("About Example", "about"),
                Entry::separator(),
                Entry::item("Quit Example", "quit").shortcut("Super+Q"),
            ],
        ),
        Entry::submenu(
            "Edit",
            vec![
                Entry::item("Copy", "copy").enabled(false),
                Entry::item("Paste_Special", "paste"),
            ],
        ),
    ]
}

fn get_layout(client: &Connection, parent: i32) -> (u32, Layout) {
    client
        .call_method(
            Some(APP),
            MENU_PATH,
            Some("com.canonical.dbusmenu"),
            "GetLayout",
            &(parent, -1, Vec::<String>::new()),
        )
        .expect("GetLayout")
        .body()
        .deserialize()
        .expect("layout")
}

fn click(client: &Connection, id: i32) {
    client
        .call_method(
            Some(APP),
            MENU_PATH,
            Some("com.canonical.dbusmenu"),
            "Event",
            &(id, "clicked", Value::from(""), 0u32),
        )
        .expect("Event");
}

fn property<'a>(properties: &'a HashMap<String, OwnedValue>, name: &str) -> Option<&'a Value<'a>> {
    properties.get(name).map(|value| &**value)
}

fn label(node: &OwnedValue) -> (i32, String) {
    let (id, properties, _): Layout = node.try_clone().unwrap().try_into().expect("node");
    let label = match property(&properties, "label") {
        Some(Value::Str(text)) => text.to_string(),
        _ => String::new(),
    };
    (id, label)
}

fn wait_for_actions(menu: &mut AppMenu) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let actions = menu.take_actions();
        if !actions.is_empty() || Instant::now() > deadline {
            return actions;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_menu_bar_reads_the_menus_and_clicks_come_back_as_actions() {
    let bus = SessionBus::start().expect("bus");
    let mut menu = AppMenu::export_on(bus.connect().expect("app"), APP, menus()).expect("export");
    let client = bus.connect().expect("client");

    let (revision, (root, _, menus)) = get_layout(&client, 0);
    assert_eq!(root, 0);
    assert_eq!(
        menus.iter().map(label).collect::<Vec<_>>(),
        [(1, "Example".to_owned()), (5, "Edit".to_owned())]
    );

    let (_, (edit, _, items)) = get_layout(&client, 5);
    assert_eq!(edit, 5);
    assert_eq!(
        items.iter().map(label).collect::<Vec<_>>(),
        [(6, "Copy".to_owned()), (7, "Paste__Special".to_owned())]
    );
    let (_, (_, quit, _)) = get_layout(&client, 4);
    assert_eq!(
        property(&quit, "shortcut").map(ToString::to_string),
        Some(r#"[["Super", "Q"]]"#.to_owned())
    );

    click(&client, 6);
    click(&client, 4);
    assert_eq!(wait_for_actions(&mut menu), ["quit"]);

    let mut changed = menus_with_copy_enabled();
    menu.update(changed.clone()).expect("update");
    let (newer, _) = get_layout(&client, 0);
    assert!(newer > revision);
    changed.truncate(1);
    menu.update(changed).expect("update");
    let (_, (_, _, menus)) = get_layout(&client, 0);
    assert_eq!(menus.len(), 1);
}

fn menus_with_copy_enabled() -> Vec<Entry> {
    let mut menus = menus();
    menus[1] = Entry::submenu("Edit", vec![Entry::item("Copy", "copy")]);
    menus
}

#[test]
fn the_process_that_claims_last_owns_the_app_menu() {
    let bus = SessionBus::start().expect("bus");
    let first = AppMenu::export_on(bus.connect().expect("first"), APP, menus()).expect("first");
    let second_connection = bus.connect().expect("second");
    let second_name = second_connection.unique_name().expect("name").to_string();
    let second = AppMenu::export_on(second_connection, APP, menus()).expect("second");
    let client = bus.connect().expect("client");
    let owner = || {
        DBusProxy::new(&client)
            .expect("proxy")
            .get_name_owner(APP.try_into().expect("name"))
            .expect("owner")
            .to_string()
    };
    assert_eq!(owner(), second_name);
    first.claim().expect("claim");
    assert_ne!(owner(), second_name);
    drop(first);
    let deadline = Instant::now() + Duration::from_secs(5);
    while owner() != second_name && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(owner(), second_name);
    drop(second);
}
