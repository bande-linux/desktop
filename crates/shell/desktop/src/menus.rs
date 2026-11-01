use bande_toolkit::ui::{Menu, MenuItem};

use crate::L10N;
use crate::model::{Kind, SortBy};

const SORTS: [(SortBy, &str, &str); 5] = [
    (SortBy::None, "sort-none", "none"),
    (SortBy::Name, "sort-name", "name"),
    (SortBy::Kind, "sort-kind", "kind"),
    (SortBy::DateAdded, "sort-date-added", "date-added"),
    (SortBy::Size, "sort-size", "size"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub count: usize,
    pub kinds: Vec<Kind>,
    pub picture: bool,
    pub apps: Vec<App>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    NewFolder,
    DesktopInfo,
    ChangeWallpaper,
    Sort(SortBy),
    CleanUp,
    ShowDesktopFolder,
    Open,
    OpenWith(String),
    Info,
    Rename,
    Duplicate,
    Copy,
    Cut,
    Paste,
    Trash,
    DeleteImmediately,
    Eject,
    SetWallpaper,
}

fn item(message: &str, action: &str) -> MenuItem {
    MenuItem::new(&L10N.tr(message), action)
}

pub fn desktop_menu(sort: SortBy, paste: bool) -> Menu {
    let sorts = SORTS
        .iter()
        .map(|(by, message, keyword)| {
            item(message, &format!("sort:{keyword}")).checked(*by == sort)
        })
        .collect();
    Menu::new(vec![
        item("new-folder", "new-folder").shortcut("Shift+Ctrl+N"),
        item("paste", "paste").shortcut("Ctrl+V").enabled(paste),
        MenuItem::separator(),
        item("get-info", "desktop-info"),
        MenuItem::separator(),
        item("change-wallpaper", "change-wallpaper"),
        MenuItem::separator(),
        MenuItem::submenu(&L10N.tr("sort-by"), Menu::new(sorts)),
        item("clean-up", "clean-up").enabled(sort == SortBy::None),
        MenuItem::separator(),
        item("show-desktop-folder", "show-desktop-folder").shortcut("Shift+Ctrl+D"),
    ])
}

pub fn items_menu(selection: &Selection) -> Menu {
    let count = selection.count;
    let volumes = selection.kinds.iter().all(|kind| *kind == Kind::Volume);
    let any_volume = selection.kinds.contains(&Kind::Volume);
    let mut items = vec![item("open", "open").shortcut("Ctrl+O")];
    if !selection.apps.is_empty() {
        let apps = selection
            .apps
            .iter()
            .map(|app| MenuItem::new(&app.name, &format!("open-with:{}", app.id)))
            .collect();
        items.push(MenuItem::submenu(&L10N.tr("open-with"), Menu::new(apps)));
    }
    items.push(MenuItem::separator());
    items.push(item("get-info", "info").shortcut("Ctrl+I"));
    items.push(
        item("rename", "rename")
            .shortcut("F2")
            .enabled(count == 1 && !any_volume),
    );
    items.push(item("duplicate", "duplicate").enabled(!any_volume));
    items.push(item("cut", "cut").shortcut("Ctrl+X").enabled(!any_volume));
    items.push(item("copy", "copy").shortcut("Ctrl+C"));
    if selection.picture && count == 1 {
        items.push(MenuItem::separator());
        items.push(item("set-wallpaper", "set-wallpaper"));
    }
    items.push(MenuItem::separator());
    if volumes {
        items.push(item("eject", "eject"));
    } else {
        let title = if count == 1 {
            L10N.tr("move-to-trash")
        } else {
            L10N.tr_args("move-items-to-trash", [("count", count.into())])
        };
        items.push(
            MenuItem::new(&title, "trash")
                .shortcut("Delete")
                .enabled(!any_volume)
                .alternate(
                    item("delete-immediately", "delete-immediately")
                        .shortcut("Shift+Delete")
                        .enabled(!any_volume),
                ),
        );
    }
    Menu::new(items)
}

pub fn command(action: &str) -> Option<Command> {
    if let Some(keyword) = action.strip_prefix("sort:") {
        return SORTS
            .iter()
            .find(|(_, _, candidate)| *candidate == keyword)
            .map(|(by, _, _)| Command::Sort(*by));
    }
    if let Some(id) = action.strip_prefix("open-with:") {
        return Some(Command::OpenWith(id.to_owned()));
    }
    Some(match action {
        "new-folder" => Command::NewFolder,
        "desktop-info" => Command::DesktopInfo,
        "change-wallpaper" => Command::ChangeWallpaper,
        "clean-up" => Command::CleanUp,
        "show-desktop-folder" => Command::ShowDesktopFolder,
        "open" => Command::Open,
        "info" => Command::Info,
        "rename" => Command::Rename,
        "duplicate" => Command::Duplicate,
        "copy" => Command::Copy,
        "cut" => Command::Cut,
        "paste" => Command::Paste,
        "trash" => Command::Trash,
        "delete-immediately" => Command::DeleteImmediately,
        "eject" => Command::Eject,
        "set-wallpaper" => Command::SetWallpaper,
        _ => return None,
    })
}
