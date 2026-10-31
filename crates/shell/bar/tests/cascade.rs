#![allow(clippy::expect_used, clippy::panic)]

use bande_bar::menus::{AppMenus, cascade};
use bande_status::Command;
use bande_status::dbusmenu::Node;
use bande_toolkit::ui::Menu;

fn node(id: i32, label: &str, children: Vec<Node>) -> Node {
    Node {
        id,
        label: label.to_owned(),
        enabled: true,
        visible: true,
        children,
        ..Node::default()
    }
}

fn titles(menu: &Menu) -> Vec<String> {
    menu.items.iter().map(|item| item.title.clone()).collect()
}

#[test]
fn an_exporting_app_gets_its_bold_app_row_then_each_menu_as_a_row_and_type_to_search() {
    let root = node(
        0,
        "",
        vec![
            node(1, "Files", vec![node(10, "About Files", Vec::new())]),
            node(2, "File", vec![node(20, "New Window", Vec::new())]),
            node(3, "Edit", vec![node(30, "Copy", Vec::new())]),
            Node {
                visible: false,
                ..node(4, "Debug", vec![node(40, "Crash", Vec::new())])
            },
        ],
    );
    let menu = cascade(&AppMenus {
        name: "Files",
        root: Some(&root),
        item: ":1.7/org/bande/MenuBar",
        actions: &[],
        windows: &[],
    });

    assert_eq!(titles(&menu), ["Files", "File", "Edit"]);
    assert!(menu.items.iter().all(|item| item.submenu.is_some()));
    assert!(menu.items[0].strong);
    let app_row = menu.items[0].submenu.as_ref().expect("the app row");
    assert_eq!(titles(app_row), ["About Files"]);
    let copy = &menu.items[2].submenu.as_ref().expect("Edit").items[0];
    assert_eq!(
        Command::parse(&copy.action),
        Some(Command::Item {
            item: ":1.7/org/bande/MenuBar".to_owned(),
            id: 30
        })
    );
    let search = menu.search.as_ref().expect("type to search");
    assert_eq!(search.hint, "Type to search");
    assert_eq!(search.shortcut, "Shift+Super+/");
    assert!(search.query.is_empty());
}

#[test]
fn an_app_without_menus_gets_the_standard_app_row_its_actions_and_its_windows() {
    let menu = cascade(&AppMenus {
        name: "Firefox",
        root: None,
        item: "",
        actions: &[("new-window".to_owned(), "New Window".to_owned())],
        windows: &[("toplevel:9".to_owned(), "Start Page".to_owned())],
    });

    assert_eq!(
        titles(&menu),
        ["Firefox", "", "New Window", "", "Start Page"]
    );
    let app_row = menu.items[0].submenu.as_ref().expect("the app row");
    assert_eq!(
        titles(app_row),
        [
            "About Firefox",
            "",
            "Hide Firefox",
            "Hide Others",
            "Show All",
            "",
            "Quit Firefox"
        ]
    );
    assert!(!app_row.items[0].enabled);
    let quit = &app_row.items[6];
    assert_eq!(quit.shortcut.as_deref(), Some("Super+Q"));
    assert_eq!(Command::parse(&quit.action), Some(Command::QuitApp));
    assert_eq!(
        Command::parse(&menu.items[2].action),
        Some(Command::AppAction("new-window".to_owned()))
    );
    assert_eq!(
        Command::parse(&menu.items[4].action),
        Some(Command::ShowWindow("toplevel:9".to_owned()))
    );
    let found = menu.found("quit");
    assert_eq!(titles(&found), ["Firefox › Quit Firefox"]);
}
