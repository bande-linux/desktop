use std::collections::HashMap;
use std::io;
use std::os::fd::BorrowedFd;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use unbound_inbox::{Inbox, Outbox};
use unbound_l10n::Localizer;
use zbus::blocking::Connection;
use zbus::fdo::RequestNameFlags;
use zbus::zvariant::{OwnedValue, Structure, Value};

pub const MENU_PATH: &str = "/org/unbound/MenuBar";
const INTERFACE: &str = "com.canonical.dbusmenu";
const ROOT: i32 = 0;
const ENGLISH: &str = include_str!("../locales/en-US/unbound-app-menu.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standard {
    Hide,
    Quit,
    Minimize,
    Zoom,
    CloseWindow,
}

const STANDARD: [(Standard, &str); 5] = [
    (Standard::Hide, "app-menu:hide"),
    (Standard::Quit, "app-menu:quit"),
    (Standard::Minimize, "app-menu:minimize"),
    (Standard::Zoom, "app-menu:zoom"),
    (Standard::CloseWindow, "app-menu:close-window"),
];

impl Standard {
    pub fn action(self) -> &'static str {
        STANDARD
            .iter()
            .find(|(standard, _)| *standard == self)
            .map_or("", |(_, action)| action)
    }

    pub fn parse(action: &str) -> Option<Self> {
        STANDARD
            .iter()
            .find(|(_, known)| *known == action)
            .map(|(standard, _)| *standard)
    }
}

pub fn app_submenu(app: &str, items: Vec<Entry>) -> Entry {
    let mut entries = items;
    if !entries.is_empty() {
        entries.push(Entry::separator());
    }
    entries.extend([
        Entry::item(
            &L10N.tr_args("hide-app", [("app", app.into())]),
            Standard::Hide.action(),
        ),
        Entry::separator(),
        Entry::item(
            &L10N.tr_args("quit-app", [("app", app.into())]),
            Standard::Quit.action(),
        )
        .shortcut("Super+Q"),
    ]);
    Entry::submenu(app, entries)
}

pub fn window_submenu(items: Vec<Entry>) -> Entry {
    let mut entries = vec![
        Entry::item(&L10N.tr("minimize"), Standard::Minimize.action()).shortcut("Super+M"),
        Entry::item(&L10N.tr("zoom"), Standard::Zoom.action()),
    ];
    if !items.is_empty() {
        entries.push(Entry::separator());
        entries.extend(items);
    }
    Entry::submenu(&L10N.tr("window"), entries)
}

pub fn close_window() -> Entry {
    Entry::item(&L10N.tr("close-window"), Standard::CloseWindow.action()).shortcut("Super+W")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    label: String,
    action: String,
    shortcut: Option<String>,
    enabled: bool,
    separator: bool,
    children: Vec<Entry>,
}

impl Entry {
    pub fn item(label: &str, action: &str) -> Self {
        Self {
            label: label.to_owned(),
            action: action.to_owned(),
            shortcut: None,
            enabled: true,
            separator: false,
            children: Vec::new(),
        }
    }

    pub fn separator() -> Self {
        Self {
            separator: true,
            ..Self::item("", "")
        }
    }

    pub fn submenu(label: &str, children: Vec<Entry>) -> Self {
        Self {
            children,
            ..Self::item(label, "")
        }
    }

    pub fn shortcut(self, shortcut: &str) -> Self {
        Self {
            shortcut: Some(shortcut.to_owned()),
            ..self
        }
    }

    pub fn enabled(self, enabled: bool) -> Self {
        Self { enabled, ..self }
    }

    fn size(&self) -> i32 {
        1 + self.children.iter().map(Entry::size).sum::<i32>()
    }

    fn properties(&self) -> HashMap<String, Value<'static>> {
        let mut properties = HashMap::new();
        if self.separator {
            properties.insert("type".to_owned(), Value::from("separator"));
            return properties;
        }
        properties.insert(
            "label".to_owned(),
            Value::from(self.label.replace('_', "__")),
        );
        if !self.enabled {
            properties.insert("enabled".to_owned(), Value::from(false));
        }
        if !self.children.is_empty() {
            properties.insert("children-display".to_owned(), Value::from("submenu"));
        }
        if let Some(shortcut) = &self.shortcut {
            let keys = shortcut.split('+').map(str::to_owned).collect::<Vec<_>>();
            properties.insert("shortcut".to_owned(), Value::from(vec![keys]));
        }
        properties
    }
}

type Layout = (i32, HashMap<String, Value<'static>>, Vec<Value<'static>>);

fn layout_of(children: &[Entry], first: i32, depth: i32) -> Vec<Value<'static>> {
    if depth == 0 {
        return Vec::new();
    }
    let mut id = first;
    let mut nodes = Vec::with_capacity(children.len());
    for child in children {
        let grandchildren = layout_of(&child.children, id + 1, depth - 1);
        nodes.push(Value::Structure(Structure::from((
            id,
            child.properties(),
            grandchildren,
        ))));
        id += child.size();
    }
    nodes
}

fn find(children: &[Entry], first: i32, target: i32) -> Option<&Entry> {
    let mut id = first;
    for child in children {
        if id == target {
            return Some(child);
        }
        if target < id + child.size() {
            return find(&child.children, id + 1, target);
        }
        id += child.size();
    }
    None
}

fn layout(menus: &[Entry], parent: i32, depth: i32) -> Option<Layout> {
    if parent == ROOT {
        let mut properties = HashMap::new();
        properties.insert("children-display".to_owned(), Value::from("submenu"));
        return Some((ROOT, properties, layout_of(menus, ROOT + 1, depth)));
    }
    let entry = find(menus, ROOT + 1, parent)?;
    Some((
        parent,
        entry.properties(),
        layout_of(&entry.children, parent + 1, depth),
    ))
}

#[derive(Default)]
struct Menus {
    revision: u32,
    entries: Vec<Entry>,
}

struct DBusMenu {
    menus: Arc<Mutex<Menus>>,
    actions: Outbox<String>,
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl DBusMenu {
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, Layout)> {
        let menus = self.menus.lock().unwrap_or_else(PoisonError::into_inner);
        layout(&menus.entries, parent_id, recursion_depth)
            .map(|layout| (menus.revision, layout))
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no menu item {parent_id}")))
    }

    fn event(&self, id: i32, event_id: String, _data: OwnedValue, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        let menus = self.menus.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = find(&menus.entries, ROOT + 1, id)
            .filter(|entry| entry.enabled && !entry.action.is_empty())
        {
            self.actions.send(entry.action.clone());
        }
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn status(&self) -> String {
        "normal".to_owned()
    }

    #[zbus(property)]
    fn text_direction(&self) -> String {
        "ltr".to_owned()
    }
}

#[derive(Debug)]
pub enum Error {
    Bus(zbus::Error),
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bus(error) => write!(f, "cannot export the menus: {error}"),
            Self::Io(error) => write!(f, "cannot export the menus: {error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<zbus::Error> for Error {
    fn from(error: zbus::Error) -> Self {
        Self::Bus(error)
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub struct AppMenu {
    connection: Connection,
    app_id: String,
    menus: Arc<Mutex<Menus>>,
    actions: Inbox<String>,
}

impl AppMenu {
    pub fn export(app_id: &str, menus: Vec<Entry>) -> Result<Self, Error> {
        Self::export_on(Connection::session()?, app_id, menus)
    }

    pub fn export_on(
        connection: Connection,
        app_id: &str,
        entries: Vec<Entry>,
    ) -> Result<Self, Error> {
        let actions = Inbox::new()?;
        let menus = Arc::new(Mutex::new(Menus {
            revision: 1,
            entries,
        }));
        connection.object_server().at(
            MENU_PATH,
            DBusMenu {
                menus: Arc::clone(&menus),
                actions: actions.outbox(),
            },
        )?;
        let menu = Self {
            connection,
            app_id: app_id.to_owned(),
            menus,
            actions,
        };
        menu.claim()?;
        Ok(menu)
    }

    pub fn claim(&self) -> Result<(), Error> {
        self.connection.request_name_with_flags(
            self.app_id.as_str(),
            RequestNameFlags::AllowReplacement | RequestNameFlags::ReplaceExisting,
        )?;
        Ok(())
    }

    pub fn update(&self, entries: Vec<Entry>) -> Result<(), Error> {
        let revision = {
            let mut menus = self.menus.lock().unwrap_or_else(PoisonError::into_inner);
            if menus.entries == entries {
                return Ok(());
            }
            menus.entries = entries;
            menus.revision += 1;
            menus.revision
        };
        self.connection.emit_signal(
            None::<&str>,
            MENU_PATH,
            INTERFACE,
            "LayoutUpdated",
            &(revision, ROOT),
        )?;
        Ok(())
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.actions.fd()
    }

    pub fn take_actions(&mut self) -> Vec<String> {
        self.actions.drain()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &[include_str!("lib.rs")]),
            Vec::<&str>::new()
        );
    }
}
