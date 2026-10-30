#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use support::{SCALES, SCHEMES, Shot, appearance_portal, shot};
use unbound_graphics::{Point, TextSystem};
use unbound_toolkit_platform::{HEADER_HEIGHT, MenuPlacement, SHEET_INSET, Window};
use unbound_widgets::{Alert, Cell, Column, Context, Menu, MenuItem, Row, Sort, Table, Ui, Widget};

const SORT_BY: i32 = 4 + 28 + 28 + 9 + 14;

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

#[test]
fn menus_tables_and_alerts_match_their_goldens_in_both_schemes_at_1x_and_1_5x() {
    let _portal = appearance_portal();
    let content = |x: i32, y: i32| (x + SHEET_INSET as i32, y + HEADER_HEIGHT as i32);
    for (dark, scheme) in SCHEMES {
        for (scale, scale_name) in SCALES {
            let suffix = format!("{scheme}-{scale_name}");
            let shot_at = |title, pointer, popups| Shot {
                title,
                scale,
                dark,
                pointer,
                popups,
            };
            shot(
                &format!("menu-submenu-{suffix}"),
                show_menu,
                shot_at("Documents", content(60, SORT_BY), 2),
            );
            shot(
                &format!("table-{suffix}"),
                processes,
                shot_at("Monitor", content(100, 60), 0),
            );
            shot(
                &format!("alert-{suffix}"),
                alert,
                shot_at("Monitor", content(10, 150), 0),
            );
        }
    }
}
