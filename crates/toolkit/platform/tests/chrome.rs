#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use support::{Shot, shot};
use unbound_graphics::{Palette, Scheme, TextSystem};
use unbound_toolkit_platform::{HEADER_HEIGHT, MenuPlacement, SHEET_INSET, Window};
use unbound_widgets::{Alert, Cell, Column, Context, Menu, MenuItem, Row, Sort, Table, Ui, Widget};

fn in_content(x: i32, y: i32, popups: usize) -> Shot {
    Shot {
        scale: 2.0,
        dark: false,
        pointer: (x + SHEET_INSET as i32, y + HEADER_HEIGHT as i32),
        popups,
    }
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
    ]);
    window
        .show_menu(
            menu,
            MenuPlacement::AtPoint(unbound_graphics::Point::new(30.0, 10.0)),
        )
        .expect("menu");
    None
}

#[test]
fn a_menu_with_an_open_submenu_matches_its_golden() {
    let sort_by = 10 + 5 + 22 + 22 + 11 + 11;
    shot("menu-submenu", show_menu, in_content(60, sort_by, 2));
}

fn processes(_window: &mut Window) -> Option<Ui> {
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
        Column::new("Process", 110.0),
        Column::new("% CPU", 60.0).numeric(),
        Column::new("Memory", 76.0).numeric(),
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
        Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light)),
    );
    ui.perform(&accesskit::ActionRequest {
        action: accesskit::Action::Focus,
        target_tree: accesskit::TreeId::ROOT,
        target_node: id.into(),
        data: None,
    });
    Some(ui)
}

#[test]
fn a_table_with_a_sorted_column_and_a_selection_matches_its_golden() {
    shot("table", processes, in_content(100, 60, 0));
}

fn alert(window: &mut Window) -> Option<Ui> {
    let mut ui = processes(window)?;
    ui.present_sheet(
        Alert::new(
            "Quit “Terminal”?",
            "Its processes will stop and unsaved work will be lost.",
            |_| {},
        )
        .cancel("Cancel", "cancel")
        .primary("Quit", "quit"),
    );
    Some(ui)
}

#[test]
fn an_alert_sheet_matches_its_golden() {
    shot("alert", alert, in_content(10, 150, 0));
}
