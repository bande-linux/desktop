use std::path::PathBuf;

use bande_app::Menus;
use bande_app_menu::{Entry, key_item};
use bande_files::{
    FilesView, GROUPS, ListColumn, group_action, list_column_action, list_column_move_action,
    list_column_title,
};
use bande_fs::SortBy;
use bande_l10n::{Text, text, tr};

fn shortcut(message: Text, keys: &str) -> Entry {
    key_item(&message.tr(), keys)
}

fn plain(message: Text, action: &str) -> Entry {
    Entry::item(&message.tr(), action)
}

fn submenu(message: Text, entries: Vec<Entry>) -> Entry {
    Entry::submenu(&message.tr(), entries)
}

fn titled(title: Option<String>, message: Text, keys: &str) -> Entry {
    let enabled = title.is_some();
    key_item(&title.unwrap_or_else(|| message.tr()), keys).enabled(enabled)
}

pub const NEW_WINDOW: &str = "new-window";
pub const RECENT_FOLDER: &str = "recent-folder:";
pub const CLEAR_RECENT_FOLDERS: &str = "clear-recent-folders";

fn recent_entries(recent_folders: &[PathBuf]) -> Vec<Entry> {
    let mut entries = recent_folders
        .iter()
        .enumerate()
        .map(|(index, folder)| {
            let name = folder.file_name().map_or_else(
                || folder.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            Entry::item(&name, &format!("{RECENT_FOLDER}{index}"))
        })
        .collect::<Vec<_>>();
    entries.push(Entry::separator());
    entries.push(
        Entry::item(&tr!("Clear Menu"), CLEAR_RECENT_FOLDERS).enabled(!recent_folders.is_empty()),
    );
    entries
}

fn group_entries(view: &FilesView) -> Vec<Entry> {
    let current = view.browser().groups();
    GROUPS
        .iter()
        .map(|(group, message)| {
            Entry::item(&message.tr(), group_action(*group)).radio(*group == current)
        })
        .collect()
}

fn sort_entries(view: &FilesView) -> Vec<Entry> {
    let current = view.browser().sort().by;
    [
        (text!("Name"), "sort:name", SortBy::Name),
        (text!("Kind"), "sort:kind", SortBy::Kind),
        (text!("Date Modified"), "sort:modified", SortBy::Modified),
        (text!("Size"), "sort:size", SortBy::Size),
    ]
    .into_iter()
    .map(|(message, action, by)| Entry::item(&message.tr(), action).radio(by == current))
    .collect()
}

fn column_entries(view: &FilesView) -> Vec<Entry> {
    let shown = view.list_columns();
    let mut entries = ListColumn::ALL
        .into_iter()
        .map(|column| {
            Entry::item(&list_column_title(column), &list_column_action(column))
                .radio(shown.contains(&column))
        })
        .collect::<Vec<_>>();
    if shown.len() > 1 {
        entries.push(Entry::separator());
    }
    let moved = |column: ListColumn, by: isize, message: Text| {
        Entry::item(
            &message.tr_args(&[(
                "column",
                &list_column_title(column) as &dyn std::fmt::Display,
            )]),
            &list_column_move_action(column, by),
        )
    };
    entries.extend(
        shown
            .iter()
            .skip(1)
            .map(|column| moved(*column, -1, text!("Move {column} Left"))),
    );
    entries.extend(
        shown
            .iter()
            .take(shown.len().saturating_sub(1))
            .map(|column| moved(*column, 1, text!("Move {column} Right"))),
    );
    entries
}

pub fn windowless_menus() -> Menus {
    Menus {
        menus: vec![submenu(
            text!("File"),
            vec![Entry::item(&tr!("New Window"), NEW_WINDOW).shortcut("Ctrl+N")],
        )],
        window: Vec::new(),
    }
}

pub fn app_menus(view: &FilesView, recent_folders: &[PathBuf]) -> Menus {
    let mut file = vec![
        shortcut(text!("New Window"), "Ctrl+N"),
        shortcut(text!("New Tab"), "Ctrl+T"),
        shortcut(text!("New Folder"), "Shift+Ctrl+N"),
        shortcut(text!("Open"), "Ctrl+O"),
        Entry::separator(),
        shortcut(text!("Get Info"), "Ctrl+I"),
        Entry::item(&tr!("Browse Backups…"), "browse-backups")
            .enabled(bande_files::backups_installed()),
        shortcut(text!("Rename"), "F2"),
        Entry::item(&tr!("Duplicate"), "duplicate"),
        Entry::item(&tr!("Make Alias"), "make-alias"),
        shortcut(text!("Add to Sidebar"), "Ctrl+D"),
    ];
    if view.terminal_installed() {
        file.push(plain(text!("Open in Terminal"), "open-in-terminal"));
    }
    file.extend([
        Entry::item(&tr!("Compress"), "compress"),
        Entry::separator(),
        shortcut(text!("Move to Trash"), "Delete"),
        shortcut(text!("Delete Immediately…"), "Shift+Delete"),
        Entry::separator(),
        plain(text!("Eject"), "eject-current"),
        shortcut(text!("Find"), "Ctrl+F"),
        Entry::separator(),
        bande_app_menu::print_item(view.can_print()),
    ]);
    let menus = vec![
        submenu(text!("File"), file),
        submenu(
            text!("Edit"),
            vec![
                titled(view.undo_title(), text!("Undo"), "Ctrl+Z"),
                titled(view.redo_title(), text!("Redo"), "Shift+Ctrl+Z"),
                Entry::separator(),
                shortcut(text!("Cut"), "Ctrl+X"),
                shortcut(text!("Copy"), "Ctrl+C"),
                shortcut(text!("Copy as Pathname"), "Shift+Ctrl+C"),
                shortcut(text!("Paste"), "Ctrl+V"),
                plain(text!("Move Item Here"), "move-here"),
                shortcut(text!("Select All"), "Ctrl+A"),
            ],
        ),
        submenu(
            text!("View"),
            vec![
                shortcut(text!("as Icons"), "Ctrl+1"),
                shortcut(text!("as List"), "Ctrl+2"),
                plain(text!("as Columns"), "columns"),
                plain(text!("as Gallery"), "gallery"),
                Entry::separator(),
                Entry::item(&tr!("Use Groups"), "use-groups")
                    .radio(view.browser().groups().is_some()),
                submenu(text!("Group By"), group_entries(view)),
                submenu(text!("Sort By"), sort_entries(view)),
                submenu(text!("Columns"), column_entries(view)),
                Entry::separator(),
                shortcut(text!("Bigger Icons"), "Ctrl+="),
                shortcut(text!("Smaller Icons"), "Ctrl+-"),
                Entry::separator(),
                shortcut(
                    match view.browser().shows_hidden() {
                        true => text!("Hide Hidden Files"),
                        false => text!("Show Hidden Files"),
                    },
                    "Ctrl+H",
                ),
                plain(text!("Show or Hide Status Bar"), "toggle-status-bar"),
                shortcut(text!("Quick Look"), "Space"),
                plain(text!("Quick Look in Full Screen"), "quick-look-full-screen"),
            ],
        ),
        submenu(
            text!("Go"),
            vec![
                shortcut(text!("Back"), "Alt+Left"),
                shortcut(text!("Forward"), "Alt+Right"),
                shortcut(text!("Enclosing Folder"), "Alt+Up"),
                Entry::separator(),
                plain(text!("Desktop"), "go:desktop"),
                plain(text!("Documents"), "go:documents"),
                plain(text!("Downloads"), "go:downloads"),
                shortcut(text!("Home"), "Alt+Home"),
                plain(text!("Recents"), "go:recents"),
                shortcut(text!("Network"), "Shift+Ctrl+K"),
                Entry::separator(),
                submenu(text!("Recent Folders"), recent_entries(recent_folders)),
                shortcut(text!("Go to Folder…"), "Ctrl+L"),
                shortcut(text!("Connect to Server…"), "Ctrl+K"),
            ],
        ),
    ];
    Menus {
        menus,
        window: vec![
            shortcut(text!("Show Previous Tab"), "Ctrl+PgUp"),
            shortcut(text!("Show Next Tab"), "Ctrl+PgDn"),
        ],
    }
}
