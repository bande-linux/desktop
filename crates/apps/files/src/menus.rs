use bande_fs::{GroupBy, SortBy};
use bande_toolkit::ui::{Menu, MenuItem};
use bande_xdg::OpenWithChoice;

use crate::L10N;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Open,
    OpenInNewTab,
    OpenInTerminal,
    NewDocument(String),
    OpenWith(String),
    OpenWithOther,
    QuickLook,
    Info,
    Rename,
    Duplicate,
    MakeAlias,
    Compress,
    Copy,
    CopyPath,
    AddToSidebar,
    Cut,
    ShowInEnclosingFolder,
    Trash,
    PutBack,
    DeleteImmediately,
    NewFolder,
    Paste,
    Sort(SortBy),
    UseGroups,
    Group(Option<GroupBy>),
    Icons,
    Columns,
    List,
    Gallery,
    ToggleHidden,
    ToggleStatusBar,
    QuickLookFullScreen,
    MoveHere,
    EjectCurrent,
    ShareMail,
    ShareTo(std::path::PathBuf),
    GoTo(Destination),
    EmptyTrash,
    OpenPlaceInNewTab,
    RemoveFromSidebar,
    RestoreSidebar,
    Eject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    Desktop,
    Documents,
    Downloads,
    Recents,
}

const SHARE_TO: &str = "share-to:";
const OPEN_WITH: &str = "open-with:";
const NEW_DOCUMENT: &str = "new-document:";

const ACTIONS: &[(&str, Command)] = &[
    ("open", Command::Open),
    ("open-in-new-tab", Command::OpenInNewTab),
    ("open-in-terminal", Command::OpenInTerminal),
    ("open-with-other", Command::OpenWithOther),
    ("open-place-in-new-tab", Command::OpenPlaceInNewTab),
    ("quick-look", Command::QuickLook),
    ("info", Command::Info),
    ("rename", Command::Rename),
    ("duplicate", Command::Duplicate),
    ("make-alias", Command::MakeAlias),
    ("compress", Command::Compress),
    ("copy", Command::Copy),
    ("copy-path", Command::CopyPath),
    ("add-to-sidebar", Command::AddToSidebar),
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
    ("use-groups", Command::UseGroups),
    ("group:none", Command::Group(None)),
    ("group:kind", Command::Group(Some(GroupBy::Kind))),
    (
        "group:application",
        Command::Group(Some(GroupBy::Application)),
    ),
    ("group:modified", Command::Group(Some(GroupBy::Modified))),
    ("group:created", Command::Group(Some(GroupBy::Created))),
    ("group:added", Command::Group(Some(GroupBy::Added))),
    ("group:size", Command::Group(Some(GroupBy::Size))),
    ("icons", Command::Icons),
    ("columns", Command::Columns),
    ("list", Command::List),
    ("gallery", Command::Gallery),
    ("toggle-hidden", Command::ToggleHidden),
    ("toggle-status-bar", Command::ToggleStatusBar),
    ("quick-look-full-screen", Command::QuickLookFullScreen),
    ("move-here", Command::MoveHere),
    ("eject-current", Command::EjectCurrent),
    ("share-mail", Command::ShareMail),
    ("go:desktop", Command::GoTo(Destination::Desktop)),
    ("go:documents", Command::GoTo(Destination::Documents)),
    ("go:downloads", Command::GoTo(Destination::Downloads)),
    ("go:recents", Command::GoTo(Destination::Recents)),
    ("empty-trash", Command::EmptyTrash),
    ("remove-from-sidebar", Command::RemoveFromSidebar),
    ("restore-sidebar", Command::RestoreSidebar),
    ("eject", Command::Eject),
];

pub const GROUPS: [(Option<GroupBy>, &str); 7] = [
    (None, "menu-group-none"),
    (Some(GroupBy::Kind), "column-kind"),
    (Some(GroupBy::Application), "menu-group-application"),
    (Some(GroupBy::Modified), "column-modified"),
    (Some(GroupBy::Created), "column-created"),
    (Some(GroupBy::Added), "menu-group-added"),
    (Some(GroupBy::Size), "column-size"),
];

pub fn group_action(group: Option<GroupBy>) -> &'static str {
    action(&Command::Group(group))
}

pub fn command(action: &str) -> Option<Command> {
    if let Some(id) = action.strip_prefix(OPEN_WITH) {
        return Some(Command::OpenWith(id.to_owned()));
    }
    if let Some(path) = action.strip_prefix(SHARE_TO) {
        return Some(Command::ShareTo(path.into()));
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
pub struct Selection {
    pub count: usize,
    pub name: String,
    pub trashed: bool,
    pub away: bool,
    pub folders: bool,
    pub files: bool,
    pub terminal: bool,
    pub apps: Vec<OpenWithChoice>,
    pub mail: bool,
    pub places: Vec<std::path::PathBuf>,
}

pub fn items_menu(selection: &Selection) -> Menu {
    let count = selection.count;
    let mut items = Vec::new();
    if selection.trashed {
        items.push(item("menu-put-back", &Command::PutBack));
        items.push(
            item("menu-delete-immediately", &Command::DeleteImmediately).shortcut("Shift+Delete"),
        );
        items.push(MenuItem::separator());
        items.push(item("menu-quick-look", &Command::QuickLook).shortcut("Space"));
        items.push(item("menu-get-info", &Command::Info).shortcut("Ctrl+I"));
        return Menu::new(items);
    }
    items.push(item("open", &Command::Open).shortcut("Ctrl+O"));
    if selection.folders {
        items.push(item("menu-open-in-new-tab", &Command::OpenInNewTab));
    }
    if selection.folders && count == 1 && selection.terminal {
        items.push(item("menu-open-in-terminal", &Command::OpenInTerminal));
    }
    if selection.files {
        let mut apps = selection
            .apps
            .iter()
            .map(|app| MenuItem::new(&app.name, &format!("{OPEN_WITH}{}", app.id)))
            .collect::<Vec<_>>();
        if !apps.is_empty() {
            apps.push(MenuItem::separator());
        }
        apps.push(item("menu-open-with-other", &Command::OpenWithOther));
        items.push(MenuItem::submenu(
            &L10N.tr("menu-open-with"),
            Menu::new(apps),
        ));
    }
    items.push(MenuItem::separator());
    items.push(item("menu-quick-look", &Command::QuickLook).shortcut("Space"));
    items.push(item("menu-get-info", &Command::Info).shortcut("Ctrl+I"));
    if selection.away {
        items.push(item(
            "menu-show-in-enclosing-folder",
            &Command::ShowInEnclosingFolder,
        ));
    }
    let mut targets = Vec::new();
    if selection.mail {
        targets.push(item("menu-share-mail", &Command::ShareMail));
    }
    targets.extend(selection.places.iter().map(|place| {
        let name = place.file_name().unwrap_or_default().to_string_lossy();
        MenuItem::new(
            &L10N.tr_args(
                "menu-share-to",
                [("place", name.as_ref().to_owned().into())],
            ),
            &format!("{SHARE_TO}{}", place.display()),
        )
    }));
    if !targets.is_empty() {
        items.push(MenuItem::submenu(
            &L10N.tr("menu-share"),
            Menu::new(targets),
        ));
    }
    items.push(MenuItem::separator());
    let rename = match count {
        1 => L10N.tr("menu-rename"),
        _ => L10N.tr_args("menu-rename-items", [("count", count.into())]),
    };
    items.push(MenuItem::new(&rename, action(&Command::Rename)).shortcut("F2"));
    items.push(item("menu-duplicate", &Command::Duplicate));
    let compress = match count {
        1 => L10N.tr_args("menu-compress", [("name", selection.name.clone().into())]),
        _ => L10N.tr_args("menu-compress-items", [("count", count.into())]),
    };
    items.push(MenuItem::new(&compress, action(&Command::Compress)));
    items.push(item("menu-cut", &Command::Cut).shortcut("Ctrl+X"));
    let copy_path = match count {
        1 => L10N.tr_args(
            "menu-copy-path-of",
            [("name", selection.name.clone().into())],
        ),
        _ => L10N.tr_args("menu-copy-paths", [("count", count.into())]),
    };
    items.push(
        item("menu-copy", &Command::Copy)
            .shortcut("Ctrl+C")
            .alternate(
                MenuItem::new(&copy_path, action(&Command::CopyPath)).shortcut("Shift+Ctrl+C"),
            ),
    );
    if selection.folders {
        items.push(item("menu-add-to-sidebar", &Command::AddToSidebar).shortcut("Ctrl+D"));
    }
    items.push(MenuItem::separator());
    let trash = if count == 1 {
        L10N.tr("menu-move-to-trash")
    } else {
        L10N.tr_args("menu-move-items-to-trash", [("count", count.into())])
    };
    items.push(
        MenuItem::new(&trash, action(&Command::Trash))
            .shortcut("Delete")
            .alternate(
                item("menu-delete-immediately", &Command::DeleteImmediately)
                    .shortcut("Shift+Delete"),
            ),
    );
    Menu::new(items)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub templates: Vec<String>,
    pub trash: bool,
    pub trash_empty: bool,
    pub paste: bool,
    pub terminal: bool,
}

pub fn folder_menu(folder: Folder) -> Menu {
    let mut items = if folder.trash {
        vec![item("menu-empty-trash", &Command::EmptyTrash).enabled(!folder.trash_empty)]
    } else {
        vec![
            item("menu-new-folder", &Command::NewFolder).shortcut("Shift+Ctrl+N"),
            item("menu-paste", &Command::Paste)
                .shortcut("Ctrl+V")
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
    items.push(MenuItem::separator());
    items.push(item("menu-get-info", &Command::Info).shortcut("Ctrl+I"));
    if !folder.trash && folder.terminal {
        items.push(item("menu-open-in-terminal", &Command::OpenInTerminal));
    }
    Menu::new(items)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Folder,
    Volume,
    Trash { empty: bool },
}

pub fn place_menu(place: Place) -> Menu {
    let mut items = vec![item("menu-open-in-new-tab", &Command::OpenPlaceInNewTab)];
    match place {
        Place::Folder => {}
        Place::Volume => {
            items.push(MenuItem::separator());
            items.push(item("menu-eject", &Command::Eject));
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
