use std::path::PathBuf;

use bande_app::Menus;
use bande_app_menu::{Entry, key_item};
use bande_files::{
    FilesView, GROUPS, L10N, ListColumn, group_action, list_column_action, list_column_move_action,
    list_column_title,
};
use bande_fs::SortBy;

fn shortcut(message: &str, keys: &str) -> Entry {
    key_item(&L10N.tr(message), keys)
}

fn plain(message: &str, action: &str) -> Entry {
    Entry::item(&L10N.tr(message), action)
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

fn group_entries(view: &FilesView) -> Vec<Entry> {
    let current = view.browser().groups();
    GROUPS
        .iter()
        .map(|(group, message)| {
            Entry::item(&L10N.tr(message), group_action(*group)).radio(*group == current)
        })
        .collect()
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
    let moved = |column: ListColumn, by: isize, message: &str| {
        Entry::item(
            &L10N.tr_args(message, [("column", list_column_title(column).into())]),
            &list_column_move_action(column, by),
        )
    };
    entries.extend(
        shown
            .iter()
            .skip(1)
            .map(|column| moved(*column, -1, "menu-move-column-left")),
    );
    entries.extend(
        shown
            .iter()
            .take(shown.len().saturating_sub(1))
            .map(|column| moved(*column, 1, "menu-move-column-right")),
    );
    entries
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
                Entry::item(&L10N.tr("menu-make-alias"), "make-alias"),
                shortcut("menu-add-to-sidebar", "Ctrl+D"),
                Entry::item(&L10N.tr("menu-open-in-terminal"), "open-in-terminal")
                    .enabled(view.terminal_installed()),
                Entry::item(&L10N.tr("menu-compress-selection"), "compress"),
                Entry::separator(),
                shortcut("menu-move-to-trash", "Delete"),
                shortcut("menu-delete-immediately", "Shift+Delete"),
                Entry::separator(),
                plain("menu-eject", "eject-current"),
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
                plain("menu-move-item-here", "move-here"),
                shortcut("menu-select-all", "Ctrl+A"),
            ],
        ),
        submenu(
            "menu-view",
            vec![
                shortcut("menu-as-icons", "Ctrl+1"),
                shortcut("menu-as-list", "Ctrl+2"),
                plain("menu-as-columns", "columns"),
                plain("menu-as-gallery", "gallery"),
                Entry::separator(),
                Entry::item(&L10N.tr("menu-use-groups"), "use-groups")
                    .radio(view.browser().groups().is_some()),
                submenu("menu-group-by", group_entries(view)),
                submenu("menu-sort-by", sort_entries(view)),
                submenu("menu-columns", column_entries(view)),
                Entry::separator(),
                shortcut("menu-bigger-icons", "Ctrl+="),
                shortcut("menu-smaller-icons", "Ctrl+-"),
                Entry::separator(),
                shortcut("menu-show-hidden-files", "Ctrl+H"),
                plain("menu-status-bar", "toggle-status-bar"),
                shortcut("menu-quick-look", "Space"),
                plain("menu-quick-look-full-screen", "quick-look-full-screen"),
            ],
        ),
        submenu(
            "menu-go",
            vec![
                shortcut("menu-back", "Alt+Left"),
                shortcut("menu-forward", "Alt+Right"),
                shortcut("menu-enclosing-folder", "Alt+Up"),
                Entry::separator(),
                plain("menu-desktop", "go:desktop"),
                plain("menu-documents", "go:documents"),
                plain("menu-downloads", "go:downloads"),
                shortcut("menu-home", "Alt+Home"),
                plain("recents", "go:recents"),
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
