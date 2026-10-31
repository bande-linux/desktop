#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use support::{Shot, appearance_portal, shot};
use unbound_graphics::{Point, TextSystem};
use unbound_toolkit_platform::{FrameRegions, HEADER_HEIGHT, MenuPlacement, SHEET_INSET, Window};
use unbound_widgets::{Alert, Cell, Column, Context, Menu, MenuItem, Row, Sort, Table, Ui, Widget};

const SORT_BY: i32 = 4 + 28 + 28 + 9 + 14;
const LARGER_SORT_BY: i32 = 4 + 37 + 37 + 9 + 18;
const OVER_CLOSE: (i32, i32) = (24, 24);
const OVER_ZOOM: (i32, i32) = (66, 24);
const AWAY: (i32, i32) = (200, 120);

fn bare(_: &mut Window) -> Option<Ui> {
    None
}

fn sidebar(window: &mut Window) -> Option<Ui> {
    window.set_frame_regions(FrameRegions {
        sidebar: 110.0,
        toolbar: 0.0,
    });
    None
}

fn show_menu(window: &mut Window) -> Option<Ui> {
    let menu = Menu::new(vec![
        MenuItem::new("Open", "open").shortcut("Super+O"),
        MenuItem::new("Rename", "rename").enabled(false),
        MenuItem::separator(),
        MenuItem::submenu(
            "Sort By",
            Menu::new(vec![
                MenuItem::new("Name", "sort-name").checked(true),
                MenuItem::new("Date Modified", "sort-date"),
            ]),
        ),
        MenuItem::new("Move to Trash", "trash").shortcut("Super+Backspace"),
        MenuItem::new("Show Inspector", "inspector").shortcut("Alt+Super+I"),
    ]);
    window
        .show_menu(menu, MenuPlacement::AtPoint(Point::new(30.0, 10.0)))
        .expect("menu");
    None
}

fn show_long_menu(window: &mut Window) -> Option<Ui> {
    let menu = Menu::new(
        (1..=30)
            .map(|index| MenuItem::new(&format!("Network {index}"), "join").checked(index == 3))
            .collect(),
    );
    window
        .show_menu(menu, MenuPlacement::AtPoint(Point::new(30.0, 10.0)))
        .expect("menu");
    None
}

fn processes(window: &mut Window) -> Option<Ui> {
    let names = [
        ("Files", "1.2", "84.3 MB", "unbound"),
        ("Terminal", "12.8", "211.0 MB", "unbound"),
        (
            "compositor with an unusually long process name",
            "4.1",
            "96.5 MB",
            "unbound",
        ),
        ("pipewire", "0.7", "12.4 MB", "unbound"),
        ("NetworkManager", "0.0", "18.9 MB", "root"),
        ("systemd", "0.1", "11.1 MB", "root"),
    ];
    let mut table = Table::new(vec![
        Column::new("Process", 120.0),
        Column::new("% CPU", 64.0).numeric(),
        Column::new("Memory", 88.0).numeric(),
        Column::new("User", 60.0),
    ]);
    table.set_rows(
        names
            .iter()
            .map(|(name, cpu, memory, user)| {
                Row::new(
                    *name,
                    vec![
                        Cell::new(*name),
                        Cell::new(*cpu),
                        Cell::new(*memory),
                        Cell::new(*user).muted(),
                    ],
                )
            })
            .collect(),
    );
    table.set_sort(Some(Sort {
        column: 1,
        ascending: false,
    }));
    table.select(&[
        "Terminal".to_owned(),
        "compositor with an unusually long process name".to_owned(),
    ]);
    let id = table.id();
    let mut ui = Ui::new(
        Box::new(table),
        Context::new(TextSystem::new(), window.appearance().palette()),
    );
    ui.perform(&accesskit::ActionRequest {
        action: accesskit::Action::Focus,
        target_tree: accesskit::TreeId::ROOT,
        target_node: id.into(),
        data: None,
    });
    Some(ui)
}

fn alert(window: &mut Window) -> Option<Ui> {
    let mut ui = processes(window)?;
    ui.present_sheet(
        Alert::new(
            "Quit “Terminal”?",
            "Its processes will stop and unsaved work will be lost.",
            |_| {},
        )
        .other("Don’t Quit", "keep")
        .cancel("Cancel", "cancel")
        .primary("Quit", "quit"),
    );
    Some(ui)
}

type Prepare = fn(&mut Window) -> Option<Ui>;

#[test]
fn the_window_frame_menus_tables_and_alerts_match_their_goldens() {
    let _portal = appearance_portal();
    let content = |x: i32, y: i32| (x + SHEET_INSET as i32, y + HEADER_HEIGHT as i32);
    let documents = |dark, pointer| Shot {
        title: "Documents",
        companion: None,
        dark,
        pointer,
        popups: 0,
        text_size: 100,
    };
    let monitor = |pointer| Shot {
        title: "Monitor",
        ..documents(false, pointer)
    };
    let shots: [(&str, Prepare, Shot); 11] = [
        ("window-frame-rest-light-1x", bare, documents(false, AWAY)),
        ("window-frame-rest-dark-1x", bare, documents(true, AWAY)),
        (
            "window-frame-close-hovered-light-1x",
            bare,
            documents(false, OVER_CLOSE),
        ),
        (
            "window-frame-tiling-menu-light-1x",
            bare,
            Shot {
                popups: 1,
                ..documents(false, OVER_ZOOM)
            },
        ),
        (
            "window-frame-inactive-light-1x",
            bare,
            Shot {
                companion: Some("Downloads"),
                ..documents(false, AWAY)
            },
        ),
        (
            "window-frame-sidebar-light-1x",
            sidebar,
            documents(false, AWAY),
        ),
        (
            "menu-submenu-light-1x",
            show_menu,
            Shot {
                popups: 2,
                ..documents(false, content(60, SORT_BY))
            },
        ),
        (
            "menu-submenu-150-light-1x",
            show_menu,
            Shot {
                popups: 2,
                text_size: 150,
                ..documents(false, content(60, LARGER_SORT_BY))
            },
        ),
        (
            "menu-long-light-1x",
            show_long_menu,
            Shot {
                title: "Networks",
                popups: 1,
                ..documents(false, content(60, 60))
            },
        ),
        ("table-light-1x", processes, monitor(content(100, 60))),
        ("alert-light-1x", alert, monitor(content(10, 150))),
    ];
    for (name, prepare, setup) in shots {
        shot(name, prepare, setup);
    }
}
