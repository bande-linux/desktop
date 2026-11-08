use bande_fs::{GroupBy, SortBy};
use bande_l10n::{Text, text, tr};
use bande_toolkit::ui::{Menu, MenuItem};
use bande_xdg::OpenWithChoice;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Open,
    OpenInNewTab,
    OpenInTerminal,
    NewDocument(String),
    OpenWith(String),
    OpenWithOther,
    QuickLook,
    LookUp,
    Info,
    BrowseBackups,
    RestoreFromBackup,
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
    ("look-up", Command::LookUp),
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
    ("browse-backups", Command::BrowseBackups),
    ("restore-from-backup", Command::RestoreFromBackup),
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

pub const GROUPS: [(Option<GroupBy>, Text); 7] = [
    (None, text!("None")),
    (Some(GroupBy::Kind), text!("Kind")),
    (Some(GroupBy::Application), text!("Application")),
    (Some(GroupBy::Modified), text!("Date Modified")),
    (Some(GroupBy::Created), text!("Date Created")),
    (Some(GroupBy::Added), text!("Date Added")),
    (Some(GroupBy::Size), text!("Size")),
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

fn item(message: Text, command: &Command) -> MenuItem {
    MenuItem::new(&message.tr(), action(command))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub count: usize,
    pub name: String,
    pub trashed: bool,
    pub in_backup: bool,
    pub can_browse_backups: bool,
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
    if selection.in_backup {
        items.push(item(text!("Restore"), &Command::RestoreFromBackup));
        items.push(MenuItem::separator());
        items.push(item(text!("Open"), &Command::Open).shortcut("Ctrl+O"));
        items.push(item(text!("Quick Look"), &Command::QuickLook).shortcut("Space"));
        items.push(item(text!("Copy"), &Command::Copy).shortcut("Ctrl+C"));
        items.push(item(text!("Get Info"), &Command::Info).shortcut("Ctrl+I"));
        return Menu::new(items);
    }
    if selection.trashed {
        items.push(item(text!("Put Back"), &Command::PutBack));
        items.push(
            item(text!("Delete Immediately…"), &Command::DeleteImmediately)
                .shortcut("Shift+Delete"),
        );
        items.push(MenuItem::separator());
        items.push(item(text!("Quick Look"), &Command::QuickLook).shortcut("Space"));
        items.push(item(text!("Get Info"), &Command::Info).shortcut("Ctrl+I"));
        return Menu::new(items);
    }
    items.push(item(text!("Open"), &Command::Open).shortcut("Ctrl+O"));
    if selection.folders {
        items.push(item(text!("Open in New Tab"), &Command::OpenInNewTab));
    }
    if selection.folders && count == 1 && selection.terminal {
        items.push(item(text!("Open in Terminal"), &Command::OpenInTerminal));
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
        apps.push(item(text!("Other…"), &Command::OpenWithOther));
        items.push(MenuItem::submenu(&tr!("Open With"), Menu::new(apps)));
    }
    items.push(MenuItem::separator());
    items.push(item(text!("Quick Look"), &Command::QuickLook).shortcut("Space"));
    items.push(item(text!("Get Info"), &Command::Info).shortcut("Ctrl+I"));
    if selection.can_browse_backups && count == 1 {
        items.push(item(text!("Browse Backups…"), &Command::BrowseBackups));
    }
    if selection.away {
        items.push(item(
            text!("Show in Enclosing Folder"),
            &Command::ShowInEnclosingFolder,
        ));
    }
    let mut targets = Vec::new();
    if selection.mail {
        targets.push(item(text!("Mail"), &Command::ShareMail));
    }
    targets.extend(selection.places.iter().map(|place| {
        let name = place.file_name().unwrap_or_default().to_string_lossy();
        MenuItem::new(
            &tr!("Copy to {place}", place = name.as_ref().to_owned()),
            &format!("{SHARE_TO}{}", place.display()),
        )
    }));
    if !targets.is_empty() {
        items.push(MenuItem::submenu(&tr!("Share"), Menu::new(targets)));
    }
    items.push(MenuItem::separator());
    let rename = match count {
        1 => tr!("Rename"),
        _ => tr!("Rename {count} Items…", count = count),
    };
    items.push(MenuItem::new(&rename, action(&Command::Rename)).shortcut("F2"));
    items.push(item(text!("Duplicate"), &Command::Duplicate));
    let compress = match count {
        1 => tr!("Compress “{name}”", name = selection.name.clone()),
        _ => tr!("Compress {count} Items", count = count),
    };
    items.push(MenuItem::new(&compress, action(&Command::Compress)));
    if count == 1 {
        let look_up = tr!("Look Up “{name}”", name = selection.name.clone());
        items.push(MenuItem::new(&look_up, action(&Command::LookUp)));
    }
    items.push(item(text!("Cut"), &Command::Cut).shortcut("Ctrl+X"));
    let copy_path = match count {
        1 => tr!("Copy “{name}” as Pathname", name = selection.name.clone()),
        _ => tr!("Copy {count} Items as Pathnames", count = count),
    };
    items.push(
        item(text!("Copy"), &Command::Copy)
            .shortcut("Ctrl+C")
            .alternate(
                MenuItem::new(&copy_path, action(&Command::CopyPath)).shortcut("Shift+Ctrl+C"),
            ),
    );
    if selection.folders {
        items.push(item(text!("Add to Sidebar"), &Command::AddToSidebar).shortcut("Ctrl+D"));
    }
    items.push(MenuItem::separator());
    let trash = if count == 1 {
        tr!("Move to Trash")
    } else {
        tr!("Move {count} Items to Trash", count = count)
    };
    items.push(
        MenuItem::new(&trash, action(&Command::Trash))
            .shortcut("Delete")
            .alternate(
                item(text!("Delete Immediately…"), &Command::DeleteImmediately)
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
    pub can_browse_backups: bool,
}

pub fn folder_menu(folder: Folder) -> Menu {
    let mut items = if folder.trash {
        vec![item(text!("Empty Trash…"), &Command::EmptyTrash).enabled(!folder.trash_empty)]
    } else {
        vec![
            item(text!("New Folder"), &Command::NewFolder).shortcut("Shift+Ctrl+N"),
            item(text!("Paste"), &Command::Paste)
                .shortcut("Ctrl+V")
                .enabled(folder.paste),
        ]
    };
    if folder.can_browse_backups && !folder.trash {
        items.push(item(text!("Browse Backups…"), &Command::BrowseBackups));
    }
    if !folder.trash && !folder.templates.is_empty() {
        let documents = folder
            .templates
            .iter()
            .map(|name| MenuItem::new(name, &format!("{NEW_DOCUMENT}{name}")))
            .collect();
        items.insert(
            1,
            MenuItem::submenu(&tr!("New Document"), Menu::new(documents)),
        );
    }
    items.push(MenuItem::separator());
    items.push(item(text!("Get Info"), &Command::Info).shortcut("Ctrl+I"));
    if !folder.trash && folder.terminal {
        items.push(item(text!("Open in Terminal"), &Command::OpenInTerminal));
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
    let mut items = vec![item(text!("Open in New Tab"), &Command::OpenPlaceInNewTab)];
    match place {
        Place::Folder => {}
        Place::Volume => {
            items.push(MenuItem::separator());
            items.push(item(text!("Eject"), &Command::Eject));
        }
        Place::Trash { empty } => {
            items.push(MenuItem::separator());
            items.push(item(text!("Empty Trash…"), &Command::EmptyTrash).enabled(!empty));
        }
    }
    items.push(MenuItem::separator());
    items.push(item(
        text!("Remove from Sidebar"),
        &Command::RemoveFromSidebar,
    ));
    Menu::new(items)
}

pub fn sidebar_menu() -> Menu {
    Menu::new(vec![item(
        text!("Restore Default Sidebar"),
        &Command::RestoreSidebar,
    )])
}
