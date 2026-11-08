use std::os::fd::BorrowedFd;

use bande_inbox::Outbox;
use futures_lite::StreamExt;
use zbus::Task;
use zbus::blocking::Connection;
use zbus::fdo::DBusProxy;
use zbus::names::WellKnownName;
use zbus::zvariant::OwnedObjectPath;

use super::{AppMenu, Entry, Error};

pub const STATUS_ACTIVATE: &str = "status-activate";
const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/org/bande/StatusMenu";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";

type ToolTip = (String, Vec<(i32, i32, Vec<u8>)>, String, String);

struct Item {
    id: String,
    title: String,
    icon: String,
    activate: Outbox<String>,
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    fn activate(&self, _x: i32, _y: i32) {
        self.activate.send(STATUS_ACTIVATE.to_owned());
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {
        self.activate.send(STATUS_ACTIVATE.to_owned());
    }

    fn context_menu(&self, _x: i32, _y: i32) {}

    fn scroll(&self, _delta: i32, _orientation: String) {}

    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        &self.id
    }

    #[zbus(property)]
    fn title(&self) -> &str {
        &self.title
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }

    #[zbus(property)]
    fn icon_name(&self) -> &str {
        &self.icon
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        Vec::new()
    }

    #[zbus(property)]
    fn tool_tip(&self) -> ToolTip {
        (String::new(), Vec::new(), self.title.clone(), String::new())
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(MENU_PATH).unwrap_or_default()
    }
}

async fn register(connection: &zbus::Connection, service: &str) -> zbus::Result<()> {
    connection
        .call_method(
            Some(WATCHER),
            WATCHER_PATH,
            Some(WATCHER),
            "RegisterStatusNotifierItem",
            &(service,),
        )
        .await
        .map(drop)
}

async fn follow_watcher(connection: zbus::Connection, service: String) {
    if let Err(error) = register(&connection, &service).await {
        tracing::warn!("no status item host yet: {error}");
    }
    let owners = match DBusProxy::new(&connection).await {
        Ok(proxy) => {
            proxy
                .receive_name_owner_changed_with_args(&[(0, WATCHER)])
                .await
        }
        Err(error) => Err(error),
    };
    let mut owners = match owners {
        Ok(owners) => owners,
        Err(error) => return tracing::warn!("cannot follow the status item host: {error}"),
    };
    while let Some(change) = owners.next().await {
        let returned = change
            .args()
            .is_ok_and(|arguments| arguments.new_owner().is_some());
        if returned && let Err(error) = register(&connection, &service).await {
            tracing::warn!("cannot show the status item: {error}");
        }
    }
}

pub struct StatusItem {
    menu: AppMenu,
    name: WellKnownName<'static>,
    _watch: Task<()>,
}

impl StatusItem {
    pub fn serve(
        connection: Connection,
        id: &str,
        title: &str,
        icon: &str,
        entries: Vec<Entry>,
    ) -> Result<Self, Error> {
        let menu = AppMenu::serve_at(connection.clone(), MENU_PATH, entries)?;
        connection.object_server().at(
            ITEM_PATH,
            Item {
                id: id.to_owned(),
                title: title.to_owned(),
                icon: icon.to_owned(),
                activate: menu.actions.outbox(),
            },
        )?;
        let name = WellKnownName::try_from(format!(
            "org.kde.StatusNotifierItem-{}-1",
            std::process::id()
        ))
        .map_err(zbus::Error::from)?;
        connection.request_name(name.clone())?;
        let inner = connection.inner();
        let watch = inner.executor().spawn(
            follow_watcher(inner.clone(), name.to_string()),
            "status item host",
        );
        Ok(Self {
            menu,
            name,
            _watch: watch,
        })
    }

    pub fn update(&self, entries: Vec<Entry>) -> Result<(), Error> {
        self.menu.update(entries)
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.menu.fd()
    }

    pub fn take_actions(&mut self) -> Vec<String> {
        self.menu.take_actions()
    }
}

impl Drop for StatusItem {
    fn drop(&mut self) {
        let connection = &self.menu.connection;
        if let Err(error) = connection.release_name(self.name.clone()) {
            tracing::warn!("cannot remove the status item: {error}");
        }
        let server = connection.object_server();
        let _ = server.remove::<Item, _>(ITEM_PATH);
        let _ = server.remove::<super::DBusMenu, _>(MENU_PATH);
    }
}
