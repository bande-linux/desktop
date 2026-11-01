use std::process;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use bande_inbox::Outbox;
use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, MessageIterator};
use zbus::fdo::RequestNameFlags;
use zbus::names::BusName;
use zbus::proxy::CacheProperties;
use zbus::zvariant::{OwnedObjectPath, Value};
use zbus::{MatchRule, Message, message};

use crate::Update;
use crate::dbusmenu::{APP_MENU_PATH, DBusMenu, Node, tree};

const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";
const WATCHER_INTERFACE: &str = "org.kde.StatusNotifierWatcher";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const MENU_INTERFACE: &str = "com.canonical.dbusmenu";
const ICON_PIXELS: i32 = 32;
const BUS: &str = "org.freedesktop.DBus";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayIcon {
    Missing,
    Named {
        name: String,
        theme_path: Option<String>,
    },
    Pixels {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayItem {
    pub id: String,
    pub title: String,
    pub icon: TrayIcon,
    pub menu: Option<String>,
    pub item_is_menu: bool,
}

#[zbus::proxy(
    interface = "org.kde.StatusNotifierWatcher",
    default_service = "org.kde.StatusNotifierWatcher",
    default_path = "/StatusNotifierWatcher",
    gen_async = false,
    blocking_name = "Watcher"
)]
trait StatusNotifierWatcher {
    fn register_status_notifier_host(&self, service: &str) -> zbus::Result<()>;

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> zbus::Result<Vec<String>>;
}

#[zbus::proxy(
    interface = "org.kde.StatusNotifierItem",
    gen_async = false,
    blocking_name = "StatusNotifierItem"
)]
trait StatusNotifierItem {
    fn activate(&self, x: i32, y: i32) -> zbus::Result<()>;

    #[zbus(property)]
    fn title(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn status(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn icon_name(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn icon_theme_path(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;

    #[zbus(property)]
    fn menu(&self) -> zbus::Result<OwnedObjectPath>;

    #[zbus(property)]
    fn item_is_menu(&self) -> zbus::Result<bool>;
}

pub fn split(id: &str) -> (&str, &str) {
    match id.find('/') {
        Some(slash) => (&id[..slash], &id[slash..]),
        None => (id, "/StatusNotifierItem"),
    }
}

pub fn pixmap_rgba(width: i32, height: i32, argb: &[u8]) -> Option<TrayIcon> {
    let (width, height) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    if Some(argb.len()) != (width as usize).checked_mul(height as usize * 4) || width == 0 {
        return None;
    }
    let rgba = argb
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|[a, r, g, b]| [*r, *g, *b, *a])
        .collect();
    Some(TrayIcon::Pixels {
        width,
        height,
        rgba,
    })
}

fn item_proxy<'a>(connection: &Connection, id: &'a str) -> zbus::Result<StatusNotifierItem<'a>> {
    let (service, path) = split(id);
    StatusNotifierItem::builder(connection)
        .destination(service)?
        .path(path)?
        .cache_properties(CacheProperties::No)
        .build()
}

fn fetch(connection: &Connection, id: &str) -> Option<TrayItem> {
    let item = item_proxy(connection, id).ok()?;
    if item.status().is_ok_and(|status| status == "Passive") {
        return None;
    }
    let named = item.icon_name().ok().filter(|name| !name.is_empty());
    let icon = match named {
        Some(name) => TrayIcon::Named {
            name,
            theme_path: item.icon_theme_path().ok().filter(|path| !path.is_empty()),
        },
        None => item
            .icon_pixmap()
            .ok()
            .and_then(|pixmaps| {
                pixmaps
                    .into_iter()
                    .min_by_key(|(width, _, _)| width.abs_diff(ICON_PIXELS))
            })
            .and_then(|(width, height, argb)| pixmap_rgba(width, height, &argb))
            .unwrap_or(TrayIcon::Missing),
    };
    let menu = item
        .menu()
        .ok()
        .filter(|path| path.as_str() != "/")
        .map(|path| format!("{}{}", split(id).0, path.as_str()));
    Some(TrayItem {
        id: id.to_owned(),
        title: item.title().unwrap_or_default(),
        icon,
        menu,
        item_is_menu: item.item_is_menu().unwrap_or(false),
    })
}

fn items(connection: &Connection) -> Vec<TrayItem> {
    Watcher::new(connection)
        .and_then(|watcher| watcher.registered_status_notifier_items())
        .unwrap_or_default()
        .iter()
        .filter_map(|id| fetch(connection, id))
        .collect()
}

fn register(connection: &Connection, name: &str) {
    if let Err(error) =
        Watcher::new(connection).and_then(|watcher| watcher.register_status_notifier_host(name))
    {
        tracing::warn!("cannot host status items: {error}");
    }
}

pub fn host(connection: &Connection, outbox: &Outbox<Update>) -> zbus::Result<()> {
    let name = format!("org.kde.StatusNotifierHost-{}", process::id());
    DBusProxy::new(connection)?.request_name(
        name.as_str().try_into()?,
        RequestNameFlags::DoNotQueue.into(),
    )?;
    let messages = MessageIterator::from(connection);
    let bus = DBusProxy::new(connection)?;
    for rule in tray_rules()? {
        bus.add_match_rule(rule)?;
    }
    let (events, received) = mpsc::channel();
    thread::spawn(move || {
        for message in messages.flatten() {
            if let Some(event) = tray_event(&message)
                && events.send(event).is_err()
            {
                break;
            }
        }
    });
    register(connection, &name);
    let mut shown = items(connection);
    outbox.send(Update::Tray(shown.clone()));
    while let Ok(first) = received.recv() {
        let mut changed = false;
        for event in std::iter::once(first).chain(received.try_iter()) {
            match event {
                TrayEvent::WatcherReturned => register(connection, &name),
                TrayEvent::AppMenuChanged => {
                    if !outbox.send(Update::AppMenuChanged) {
                        return Ok(());
                    }
                    continue;
                }
                TrayEvent::Items => {}
            }
            changed = true;
        }
        if !changed {
            continue;
        }
        let current = items(connection);
        if current != shown {
            shown = current;
            if !outbox.send(Update::Tray(shown.clone())) {
                return Ok(());
            }
        }
    }
    Ok(())
}

enum TrayEvent {
    Items,
    WatcherReturned,
    AppMenuChanged,
}

fn tray_rules() -> zbus::Result<[MatchRule<'static>; 4]> {
    let signal = || MatchRule::builder().msg_type(message::Type::Signal);
    Ok([
        signal().interface(ITEM_INTERFACE)?.build(),
        signal().interface(WATCHER_INTERFACE)?.build(),
        signal()
            .sender(BUS)?
            .interface(BUS)?
            .member("NameOwnerChanged")?
            .add_arg(WATCHER)?
            .build(),
        signal()
            .interface(MENU_INTERFACE)?
            .member("LayoutUpdated")?
            .path(APP_MENU_PATH)?
            .build(),
    ])
}

fn tray_event(message: &Message) -> Option<TrayEvent> {
    let header = message.header();
    if header.message_type() != message::Type::Signal {
        return None;
    }
    let member = header.member()?.as_str();
    match header.interface()?.as_str() {
        ITEM_INTERFACE | WATCHER_INTERFACE => Some(TrayEvent::Items),
        BUS if member == "NameOwnerChanged" => message
            .body()
            .deserialize::<(String, String, String)>()
            .is_ok_and(|(owner_of, _, new)| owner_of == WATCHER && !new.is_empty())
            .then_some(TrayEvent::WatcherReturned),
        MENU_INTERFACE
            if member == "LayoutUpdated"
                && header
                    .path()
                    .is_some_and(|path| path.as_str() == APP_MENU_PATH) =>
        {
            Some(TrayEvent::AppMenuChanged)
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Errand {
    ShowMenu { menu: String, parent: i32 },
    Clicked { menu: String, id: i32 },
    Activate { item: String, x: i32, y: i32 },
    AppMenu(String),
}

fn dbus_menu<'a>(connection: &Connection, key: &'a str) -> zbus::Result<DBusMenu<'a>> {
    let (service, path) = split(key);
    DBusMenu::builder(connection)
        .destination(service)?
        .path(path)?
        .cache_properties(CacheProperties::No)
        .build()
}

fn layout(connection: &Connection, key: &str, parent: i32) -> zbus::Result<Node> {
    let proxy = dbus_menu(connection, key)?;
    let _ = proxy.about_to_show(parent);
    let (_, root) = proxy.get_layout(parent, -1, &[])?;
    Ok(tree(root))
}

fn without_repeated_app_menus(errands: impl Iterator<Item = Errand>) -> Vec<Errand> {
    let mut kept: Vec<Errand> = Vec::new();
    for errand in errands {
        if matches!(errand, Errand::AppMenu(_)) && kept.contains(&errand) {
            continue;
        }
        kept.push(errand);
    }
    kept
}

pub fn run_errands(connection: &Connection, errands: &Receiver<Errand>, outbox: &Outbox<Update>) {
    while let Ok(next) = errands.recv() {
        let queued = std::iter::once(next).chain(errands.try_iter());
        for errand in without_repeated_app_menus(queued) {
            run_errand(connection, errand, outbox);
        }
    }
}

fn run_errand(connection: &Connection, errand: Errand, outbox: &Outbox<Update>) {
    let result = match errand {
        Errand::ShowMenu { menu, parent } => layout(connection, &menu, parent).map(|root| {
            outbox.send(Update::Menu { menu, root });
        }),
        Errand::Clicked { menu, id } => dbus_menu(connection, &menu)
            .and_then(|proxy| proxy.event(id, "clicked", &Value::from(""), 0)),
        Errand::Activate { item, x, y } => {
            item_proxy(connection, &item).and_then(|proxy| proxy.activate(x, y))
        }
        Errand::AppMenu(app) => {
            let key = format!("{app}{APP_MENU_PATH}");
            let running = BusName::try_from(app.as_str()).is_ok_and(|name| {
                DBusProxy::new(connection)
                    .and_then(|bus| Ok(bus.name_has_owner(name)?))
                    .unwrap_or(false)
            });
            let root = running
                .then(|| dbus_menu(connection, &key).and_then(|proxy| proxy.get_layout(0, -1, &[])))
                .and_then(Result::ok)
                .map(|(_, root)| tree(root));
            outbox.send(Update::AppMenu { app, root });
            Ok(())
        }
    };
    if let Err(error) = result {
        tracing::warn!("a status item did not answer: {error}");
    }
}

