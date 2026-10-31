use unbound_fs::SortBy;
use unbound_toolkit::ui::{Menu, MenuItem};

use crate::L10N;
use crate::views::ViewMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Open,
    OpenInNewTab,
    OpenInTerminal,
    NewDocument(String),
    OpenWith(String),
    QuickLook,
    Info,
    Rename,
    Duplicate,
    Compress,
    Copy,
    Cut,
    ShowInEnclosingFolder,
    Trash,
    PutBack,
    DeleteImmediately,
    NewFolder,
    Paste,
    Sort(SortBy),
    Icons,
    Columns,
    List,
    ToggleHidden,
    EmptyTrash,
    OpenPlace,
    OpenPlaceInNewTab,
    RemoveFromSidebar,
    RestoreSidebar,
    Eject,
}

const OPEN_WITH: &str = "open-with:";
const NEW_DOCUMENT: &str = "new-document:";

const ACTIONS: &[(&str, Command)] = &[
    ("open", Command::Open),
    ("open-in-new-tab", Command::OpenInNewTab),
    ("open-in-terminal", Command::OpenInTerminal),
    ("open-place-in-new-tab", Command::OpenPlaceInNewTab),
    ("quick-look", Command::QuickLook),
    ("info", Command::Info),
    ("rename", Command::Rename),
    ("duplicate", Command::Duplicate),
    ("compress", Command::Compress),
    ("copy", Command::Copy),
    ("cut", Command::Cut),
    ("show-in-enclosing-folder", Command::ShowInEnclosingFolder),
    ("trash", Command::Trash),
    ("put-back", Command::PutBack),
    ("delete-immediately", Command::DeleteImmediately),
    ("new-folder", Command::NewFolder),
    ("paste", Command::Paste),
    ("sort:name", Command::Sort(SortBy::Name)),
    ("sort:modified", Command::Sort(SortBy::Modified)),
    ("sort:size", Command::Sort(SortBy::Size)),
    ("sort:kind", Command::Sort(SortBy::Kind)),
    ("icons", Command::Icons),
    ("columns", Command::Columns),
    ("list", Command::List),
    ("toggle-hidden", Command::ToggleHidden),
    ("empty-trash", Command::EmptyTrash),
    ("open-place", Command::OpenPlace),
    ("remove-from-sidebar", Command::RemoveFromSidebar),
    ("restore-sidebar", Command::RestoreSidebar),
    ("eject", Command::Eject),
];

const SORTS: [(SortBy, &str); 4] = [
    (SortBy::Name, "column-name"),
    (SortBy::Kind, "column-kind"),
    (SortBy::Modified, "column-modified"),
    (SortBy::Size, "column-size"),
];

pub fn command(action: &str) -> Option<Command> {
    if let Some(id) = action.strip_prefix(OPEN_WITH) {
        return Some(Command::OpenWith(id.to_owned()));
    }
    if let Some(name) = action.strip_prefix(NEW_DOCUMENT) {
        return Some(Command::NewDocument(name.to_owned()));
    }
    ACTIONS
        .iter()
        .find(|(known, _)| *known == action)
        .map(|(_, command)| command.clone())
}

fn action(command: &Command) -> &'static str {
    ACTIONS
        .iter()
        .find(|(_, known)| known == command)
        .map_or("", |(action, _)| action)
}

fn item(message: &str, command: &Command) -> MenuItem {
    MenuItem::new(&L10N.tr(message), action(command))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub count: usize,
    pub name: String,
    pub trashed: bool,
    pub searching: bool,
    pub folders: bool,
    pub apps: Vec<App>,
}

pub fn items_menu(selection: &Selection) -> Menu {
    let count = selection.count;
    let mut items = Vec::new();
    if selection.trashed {
        items.push(item("menu-put-back", &Command::PutBack).shortcut("Super+Backspace"));
        items.push(
            item("menu-delete-immediately", &Command::DeleteImmediately)
                .shortcut("Alt+Super+Backspace"),
        );
        items.push(MenuItem::separator());
        items.push(item("menu-quick-look", &Command::QuickLook).enabled(count == 1));
        items.push(item("menu-get-info", &Command::Info).shortcut("Super+I"));
        return Menu::new(items);
    }
    items.push(item("open", &Command::Open).shortcut("Super+Down"));
    if selection.folders {
        items.push(item("menu-open-in-new-tab", &Command::OpenInNewTab));
    }
    if selection.folders && count == 1 {
        items.push(item("menu-open-in-terminal", &Command::OpenInTerminal));
    }
    if !selection.apps.is_empty() {
        let apps = selection
            .apps
            .iter()
            .map(|app| MenuItem::new(&app.name, &format!("{OPEN_WITH}{}", app.id)))
            .collect();
        items.push(MenuItem::submenu(
            &L10N.tr("menu-open-with"),
            Menu::new(apps),
        ));
    }
    items.push(MenuItem::separator());
    items.push(
        item("menu-quick-look", &Command::QuickLook)
            .shortcut("Space")
            .enabled(count == 1),
    );
    items.push(item("menu-get-info", &Command::Info).shortcut("Super+I"));
    if selection.searching {
        items.push(item(
            "menu-show-in-enclosing-folder",
            &Command::ShowInEnclosingFolder,
        ));
    }
    items.push(MenuItem::separator());
    items.push(
        item("menu-rename", &Command::Rename)
            .shortcut("F2")
            .enabled(count == 1),
    );
    items.push(item("menu-duplicate", &Command::Duplicate).shortcut("Super+D"));
    let compress = match count {
        1 => L10N.tr_args("menu-compress", [("name", selection.name.clone().into())]),
        _ => L10N.tr_args("menu-compress-items", [("count", count.into())]),
    };
    items.push(MenuItem::new(&compress, action(&Command::Compress)));
    items.push(item("menu-cut", &Command::Cut).shortcut("Super+X"));
    items.push(item("menu-copy", &Command::Copy).shortcut("Super+C"));
    items.push(MenuItem::separator());
    let trash = if count == 1 {
        L10N.tr("menu-move-to-trash")
    } else {
        L10N.tr_args("menu-move-items-to-trash", [("count", count.into())])
    };
    items.push(
        MenuItem::new(&trash, action(&Command::Trash))
            .shortcut("Super+Backspace")
            .alternate(
                item("menu-delete-immediately", &Command::DeleteImmediately)
                    .shortcut("Alt+Super+Backspace"),
            ),
    );
    Menu::new(items)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub templates: Vec<String>,
    pub trash: bool,
    pub trash_empty: bool,
    pub sort: SortBy,
    pub mode: ViewMode,
    pub hidden_shown: bool,
    pub paste: bool,
}

pub fn folder_menu(folder: Folder) -> Menu {
    let mut items = if folder.trash {
        vec![
            item("menu-empty-trash", &Command::EmptyTrash)
                .shortcut("Shift+Super+Backspace")
                .enabled(!folder.trash_empty),
        ]
    } else {
        vec![
            item("menu-new-folder", &Command::NewFolder).shortcut("Shift+Super+N"),
            item("menu-paste", &Command::Paste)
                .shortcut("Super+V")
                .enabled(folder.paste),
        ]
    };
    if !folder.trash && !folder.templates.is_empty() {
        let documents = folder
            .templates
            .iter()
            .map(|name| MenuItem::new(name, &format!("{NEW_DOCUMENT}{name}")))
            .collect();
        items.insert(
            1,
            MenuItem::submenu(&L10N.tr("menu-new-document"), Menu::new(documents)),
        );
    }
    items.push(item("menu-get-info", &Command::Info).shortcut("Super+I"));
    if !folder.trash {
        items.push(item("menu-open-in-terminal", &Command::OpenInTerminal));
    }
    let sorts = SORTS
        .iter()
        .map(|(by, message)| item(message, &Command::Sort(*by)).checked(*by == folder.sort))
        .collect();
    items.push(MenuItem::separator());
    items.push(MenuItem::submenu(
        &L10N.tr("menu-sort-by"),
        Menu::new(sorts),
    ));
    items.push(MenuItem::separator());
    for (message, command, keys, mode) in [
        ("menu-as-icons", Command::Icons, "Super+1", ViewMode::Icons),
        ("menu-as-list", Command::List, "Super+2", ViewMode::List),
        (
            "menu-as-columns",
            Command::Columns,
            "Super+3",
            ViewMode::Columns,
        ),
    ] {
        items.push(
            item(message, &command)
                .shortcut(keys)
                .checked(folder.mode == mode),
        );
    }
    let hidden = if folder.hidden_shown {
        "menu-hide-hidden-files"
    } else {
        "menu-show-hidden-files"
    };
    items.push(item(hidden, &Command::ToggleHidden).shortcut("Shift+Super+."));
    Menu::new(items)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Folder,
    Volume,
    Trash { empty: bool },
}

pub fn place_menu(place: Place) -> Menu {
    let mut items = vec![
        item("open", &Command::OpenPlace),
        item("menu-open-in-new-tab", &Command::OpenPlaceInNewTab),
    ];
    match place {
        Place::Folder => {}
        Place::Volume => {
            items.push(MenuItem::separator());
            items.push(item("menu-eject", &Command::Eject).shortcut("Super+E"));
        }
        Place::Trash { empty } => {
            items.push(MenuItem::separator());
            items.push(item("menu-empty-trash", &Command::EmptyTrash).enabled(!empty));
        }
    }
    items.push(MenuItem::separator());
    items.push(item(
        "menu-remove-from-sidebar",
        &Command::RemoveFromSidebar,
    ));
    Menu::new(items)
}

pub fn sidebar_menu() -> Menu {
    Menu::new(vec![item("menu-restore-sidebar", &Command::RestoreSidebar)])
}
