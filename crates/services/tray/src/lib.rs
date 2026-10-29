use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use zbus::blocking::Connection;
use zbus::blocking::fdo::DBusProxy;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;

pub const BUS_NAME: &str = "org.kde.StatusNotifierWatcher";
pub const OBJECT_PATH: &str = "/StatusNotifierWatcher";
const DEFAULT_ITEM_PATH: &str = "/StatusNotifierItem";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    owner: String,
    id: String,
}

#[derive(Default)]
struct Registry {
    items: Vec<Item>,
    hosts: Vec<String>,
}

type Shared = Arc<Mutex<Registry>>;

fn lock(registry: &Shared) -> MutexGuard<'_, Registry> {
    registry.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct Watcher {
    _connection: Connection,
}

impl Watcher {
    pub fn start(connection: &Connection) -> zbus::Result<Self> {
        let registry = Shared::default();
        let owners = DBusProxy::new(connection)?.receive_name_owner_changed()?;
        connection.object_server().at(
            OBJECT_PATH,
            StatusNotifierWatcher {
                registry: Arc::clone(&registry),
            },
        )?;
        let interface = connection
            .object_server()
            .interface::<_, StatusNotifierWatcher>(OBJECT_PATH)?;
        thread::spawn(move || {
            for change in owners {
                let Ok(arguments) = change.args() else {
                    continue;
                };
                if arguments.new_owner().is_some() {
                    continue;
                }
                let gone = arguments.name().as_str().to_owned();
                let (items, hosts) = {
                    let mut registry = lock(&registry);
                    let items: Vec<String> = registry
                        .items
                        .iter()
                        .filter(|item| item.owner == gone)
                        .map(|item| item.id.clone())
                        .collect();
                    registry.items.retain(|item| item.owner != gone);
                    let had_host = registry.hosts.contains(&gone);
                    registry.hosts.retain(|host| *host != gone);
                    (items, had_host && registry.hosts.is_empty())
                };
                let emitter = interface.signal_emitter();
                let watcher = interface.get();
                zbus::block_on(async {
                    for id in &items {
                        let _ =
                            StatusNotifierWatcher::status_notifier_item_unregistered(emitter, id)
                                .await;
                    }
                    if !items.is_empty() {
                        let _ = watcher
                            .registered_status_notifier_items_changed(emitter)
                            .await;
                    }
                    if hosts {
                        let _ =
                            StatusNotifierWatcher::status_notifier_host_unregistered(emitter).await;
                        let _ = watcher
                            .is_status_notifier_host_registered_changed(emitter)
                            .await;
                    }
                });
            }
        });
        connection.request_name(BUS_NAME)?;
        Ok(Self {
            _connection: connection.clone(),
        })
    }
}

fn sender(header: &Header<'_>) -> zbus::fdo::Result<String> {
    header
        .sender()
        .map(ToString::to_string)
        .ok_or_else(|| zbus::fdo::Error::InvalidArgs("the call has no sender".to_owned()))
}

fn item_id(service: &str, sender: &str) -> String {
    if service.starts_with('/') {
        format!("{sender}{service}")
    } else {
        format!("{service}{DEFAULT_ITEM_PATH}")
    }
}

struct StatusNotifierWatcher {
    registry: Shared,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl StatusNotifierWatcher {
    async fn register_status_notifier_item(
        &self,
        service: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let caller = sender(&header)?;
        let owner = if service.starts_with('/') {
            caller.clone()
        } else {
            zbus::fdo::DBusProxy::new(connection)
                .await?
                .get_name_owner(
                    zbus::names::BusName::try_from(service.as_str()).map_err(|_| {
                        zbus::fdo::Error::InvalidArgs(format!(
                            "{service} is not a bus name or path"
                        ))
                    })?,
                )
                .await?
                .to_string()
        };
        let id = item_id(&service, &caller);
        let added = {
            let mut registry = lock(&self.registry);
            let known = registry.items.iter().any(|item| item.id == id);
            if !known {
                registry.items.push(Item {
                    owner,
                    id: id.clone(),
                });
            }
            !known
        };
        if added {
            Self::status_notifier_item_registered(&emitter, &id).await?;
            self.registered_status_notifier_items_changed(&emitter)
                .await?;
        }
        Ok(())
    }

    async fn register_status_notifier_host(
        &self,
        _service: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let caller = sender(&header)?;
        let first = {
            let mut registry = lock(&self.registry);
            let first = registry.hosts.is_empty();
            if !registry.hosts.contains(&caller) {
                registry.hosts.push(caller);
            }
            first
        };
        if first {
            Self::status_notifier_host_registered(&emitter).await?;
            self.is_status_notifier_host_registered_changed(&emitter)
                .await?;
        }
        Ok(())
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        lock(&self.registry)
            .items
            .iter()
            .map(|item| item.id.clone())
            .collect()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        !lock(&self.registry).hosts.is_empty()
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_unregistered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}
