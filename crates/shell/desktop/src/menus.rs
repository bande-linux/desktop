use bande_l10n::{Text, text, tr};

use crate::model::{Kind, SortBy};
use bande_toolkit::ui::{Menu, MenuItem};
use bande_xdg::OpenWithChoice;

const SORTS: [(SortBy, Text, &str); 5] = [
    (SortBy::None, text!("None"), "none"),
    (SortBy::Name, text!("Name"), "name"),
    (SortBy::Kind, text!("Kind"), "kind"),
    (SortBy::DateAdded, text!("Date Added"), "date-added"),
    (SortBy::Size, text!("Size"), "size"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub count: usize,
    pub kinds: Vec<Kind>,
    pub picture: bool,
    pub apps: Vec<OpenWithChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    NewFolder,
    ChangeWallpaper,
    Sort(SortBy),
    CleanUp,
    Open,
    QuickLook,
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

fn item(message: Text, action: &str) -> MenuItem {
    MenuItem::new(&message.tr(), action)
}

pub fn desktop_menu(sort: SortBy, paste: bool) -> Menu {
    let sorts = SORTS
        .iter()
        .map(|(by, message, keyword)| {
            item(*message, &format!("sort:{keyword}")).checked(*by == sort)
        })
        .collect();
    Menu::new(vec![
        item(text!("New Folder"), "new-folder").shortcut("Shift+Ctrl+N"),
        item(text!("Paste"), "paste")
            .shortcut("Ctrl+V")
            .enabled(paste),
        MenuItem::separator(),
        item(text!("Change Wallpaper…"), "change-wallpaper"),
        MenuItem::separator(),
        MenuItem::submenu(&tr!("Sort By"), Menu::new(sorts)),
        item(text!("Clean Up"), "clean-up").enabled(sort == SortBy::None),
    ])
}

pub fn items_menu(selection: &Selection) -> Menu {
    let count = selection.count;
    let volumes = selection.kinds.iter().all(|kind| *kind == Kind::Volume);
    let any_volume = selection.kinds.contains(&Kind::Volume);
    let mut items = vec![item(text!("Open"), "open").shortcut("Ctrl+O")];
    if !selection.apps.is_empty() {
        let apps = selection
            .apps
            .iter()
            .map(|app| MenuItem::new(&app.name, &format!("open-with:{}", app.id)))
            .collect();
        items.push(MenuItem::submenu(&tr!("Open With"), Menu::new(apps)));
    }
    items.push(MenuItem::separator());
    items.push(item(text!("Quick Look"), "quick-look").shortcut("Space"));
    items.push(item(text!("Get Info"), "info").shortcut("Ctrl+I"));
    items.push(
        item(text!("Rename"), "rename")
            .shortcut("F2")
            .enabled(count == 1 && !any_volume),
    );
    items.push(item(text!("Duplicate"), "duplicate").enabled(!any_volume));
    items.push(
        item(text!("Cut"), "cut")
            .shortcut("Ctrl+X")
            .enabled(!any_volume),
    );
    items.push(item(text!("Copy"), "copy").shortcut("Ctrl+C"));
    if selection.picture {
        items.push(MenuItem::separator());
        items.push(item(text!("Set as Wallpaper"), "set-wallpaper"));
    }
    items.push(MenuItem::separator());
    if volumes {
        items.push(item(text!("Eject"), "eject"));
    } else {
        let title = if count == 1 {
            tr!("Move to Trash")
        } else {
            tr!("Move {count} Items to Trash", count = count)
        };
        items.push(
            MenuItem::new(&title, "trash")
                .shortcut("Delete")
                .enabled(!any_volume)
                .alternate(
                    item(text!("Delete Immediately…"), "delete-immediately")
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
        "change-wallpaper" => Command::ChangeWallpaper,
        "clean-up" => Command::CleanUp,
        "open" => Command::Open,
        "quick-look" => Command::QuickLook,
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
