use std::path::PathBuf;

use bande_app::Menus;
use bande_app_menu::{Entry, key_item};
use bande_files::{FilesView, L10N};
use bande_fs::SortBy;

fn shortcut(message: &str, keys: &str) -> Entry {
    key_item(&L10N.tr(message), keys)
}

fn submenu(message: &str, entries: Vec<Entry>) -> Entry {
    Entry::submenu(&L10N.tr(message), entries)
}

fn titled(title: Option<String>, message: &str, keys: &str) -> Entry {
    let enabled = title.is_some();
    key_item(&title.unwrap_or_else(|| L10N.tr(message)), keys).enabled(enabled)
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
        Entry::item(&L10N.tr("menu-clear-menu"), CLEAR_RECENT_FOLDERS)
            .enabled(!recent_folders.is_empty()),
    );
    entries
}

fn sort_entries(view: &FilesView) -> Vec<Entry> {
    let current = view.browser().sort().by;
    [
        ("column-name", "sort:name", SortBy::Name),
        ("column-kind", "sort:kind", SortBy::Kind),
        ("column-modified", "sort:modified", SortBy::Modified),
        ("column-size", "sort:size", SortBy::Size),
    ]
    .into_iter()
    .map(|(message, action, by)| Entry::item(&L10N.tr(message), action).radio(by == current))
    .collect()
}

pub fn windowless_menus() -> Menus {
    Menus {
        menus: vec![submenu(
            "menu-file",
            vec![Entry::item(&L10N.tr("menu-new-window"), NEW_WINDOW).shortcut("Ctrl+N")],
        )],
        window: Vec::new(),
    }
}

pub fn app_menus(view: &FilesView, recent_folders: &[PathBuf]) -> Menus {
    let menus = vec![
        submenu(
            "menu-file",
            vec![
                shortcut("menu-new-window", "Ctrl+N"),
                shortcut("menu-new-tab", "Ctrl+T"),
                shortcut("menu-new-folder", "Shift+Ctrl+N"),
                shortcut("menu-open", "Ctrl+O"),
                Entry::separator(),
                shortcut("menu-get-info", "Ctrl+I"),
                shortcut("menu-rename", "F2"),
                Entry::item(&L10N.tr("menu-duplicate"), "duplicate"),
                shortcut("menu-add-to-sidebar", "Ctrl+D"),
                Entry::item(&L10N.tr("menu-open-in-terminal"), "open-in-terminal")
                    .enabled(view.terminal_installed()),
                Entry::item(&L10N.tr("menu-compress-selection"), "compress"),
                Entry::separator(),
                shortcut("menu-move-to-trash", "Delete"),
                shortcut("menu-delete-immediately", "Shift+Delete"),
                Entry::separator(),
                shortcut("menu-eject", "Ctrl+E"),
                shortcut("menu-find", "Ctrl+F"),
            ],
        ),
        submenu(
            "menu-edit",
            vec![
                titled(view.undo_title(), "menu-undo", "Ctrl+Z"),
                titled(view.redo_title(), "menu-redo", "Shift+Ctrl+Z"),
                Entry::separator(),
                shortcut("menu-cut", "Ctrl+X"),
                shortcut("menu-copy", "Ctrl+C"),
                shortcut("menu-copy-path", "Shift+Ctrl+C"),
                shortcut("menu-paste", "Ctrl+V"),
                shortcut("menu-move-item-here", "Shift+Ctrl+V"),
                shortcut("menu-select-all", "Ctrl+A"),
            ],
        ),
        submenu(
            "menu-view",
            vec![
                shortcut("menu-as-icons", "Ctrl+1"),
                shortcut("menu-as-list", "Ctrl+2"),
                shortcut("menu-as-columns", "Ctrl+3"),
                Entry::separator(),
                submenu("menu-sort-by", sort_entries(view)),
                Entry::separator(),
                shortcut("menu-bigger-icons", "Ctrl+="),
                shortcut("menu-smaller-icons", "Ctrl+-"),
                Entry::separator(),
                shortcut("menu-show-hidden-files", "Ctrl+H"),
                shortcut("menu-status-bar", "Ctrl+/"),
                shortcut("menu-quick-look", "Space"),
                shortcut("menu-quick-look-full-screen", "F11"),
            ],
        ),
        submenu(
            "menu-go",
            vec![
                shortcut("menu-back", "Alt+Left"),
                shortcut("menu-forward", "Alt+Right"),
                shortcut("menu-enclosing-folder", "Alt+Up"),
                Entry::separator(),
                shortcut("menu-desktop", "Shift+Ctrl+D"),
                shortcut("menu-documents", "Shift+Ctrl+O"),
                shortcut("menu-downloads", "Shift+Ctrl+L"),
                shortcut("menu-home", "Alt+Home"),
                shortcut("recents", "Shift+Ctrl+F"),
                Entry::separator(),
                submenu("menu-recent-folders", recent_entries(recent_folders)),
                shortcut("menu-go-to-folder", "Ctrl+L"),
            ],
        ),
    ];
    Menus {
        menus,
        window: vec![
            shortcut("menu-previous-tab", "Ctrl+PgUp"),
            shortcut("menu-next-tab", "Ctrl+PgDn"),
        ],
    }
}
