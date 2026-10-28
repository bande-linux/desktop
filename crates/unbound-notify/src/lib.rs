use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedValue;

pub const BUS_NAME: &str = "org.freedesktop.Notifications";
pub const OBJECT_PATH: &str = "/org/freedesktop/Notifications";
pub const SPEC_VERSION: &str = "1.2";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    Expired = 1,
    Dismissed = 2,
    Closed = 3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<(String, String)>,
    pub resident: bool,
}

#[derive(Default)]
struct Registry {
    last_id: u32,
    generation: u64,
    active: BTreeMap<u32, (Notification, u64)>,
}

impl Registry {
    fn fresh_id(&mut self) -> u32 {
        loop {
            self.last_id = self.last_id.wrapping_add(1);
            if self.last_id != 0 && !self.active.contains_key(&self.last_id) {
                return self.last_id;
            }
        }
    }
}

type Shared = Arc<Mutex<Registry>>;

fn lock(registry: &Shared) -> MutexGuard<'_, Registry> {
    registry.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct NotificationServer {
    connection: Connection,
    registry: Shared,
}

impl NotificationServer {
    pub fn start(connection: &Connection) -> zbus::Result<Self> {
        let registry = Shared::default();
        connection.object_server().at(
            OBJECT_PATH,
            Notifications {
                registry: Arc::clone(&registry),
                connection: connection.clone(),
            },
        )?;
        connection.request_name(BUS_NAME)?;
        Ok(Self {
            connection: connection.clone(),
            registry,
        })
    }

    pub fn active(&self) -> Vec<Notification> {
        lock(&self.registry)
            .active
            .values()
            .map(|(notification, _)| notification.clone())
            .collect()
    }

    pub fn dismiss(&self, id: u32) -> zbus::Result<bool> {
        if lock(&self.registry).active.remove(&id).is_none() {
            return Ok(false);
        }
        emit_closed(&self.connection, id, CloseReason::Dismissed)?;
        Ok(true)
    }

    pub fn invoke_action(&self, id: u32, action_key: &str) -> zbus::Result<bool> {
        let resident = {
            let mut registry = lock(&self.registry);
            let Some((notification, _)) = registry.active.get(&id) else {
                return Ok(false);
            };
            if !notification
                .actions
                .iter()
                .any(|(key, _)| key == action_key)
            {
                return Ok(false);
            }
            let resident = notification.resident;
            if !resident {
                registry.active.remove(&id);
            }
            resident
        };
        let emitter = SignalEmitter::new(self.connection.inner(), OBJECT_PATH)?;
        zbus::block_on(Notifications::action_invoked(&emitter, id, action_key))?;
        if !resident {
            emit_closed(&self.connection, id, CloseReason::Dismissed)?;
        }
        Ok(true)
    }
}

fn emit_closed(connection: &Connection, id: u32, reason: CloseReason) -> zbus::Result<()> {
    let emitter = SignalEmitter::new(connection.inner(), OBJECT_PATH)?;
    zbus::block_on(Notifications::notification_closed(
        &emitter,
        id,
        reason as u32,
    ))
}

fn schedule_expiry(
    connection: Connection,
    registry: Shared,
    id: u32,
    generation: u64,
    after: Duration,
) {
    thread::spawn(move || {
        thread::sleep(after);
        let expired = {
            let mut registry = lock(&registry);
            let current = registry.active.get(&id).map(|(_, current)| *current);
            current == Some(generation) && registry.active.remove(&id).is_some()
        };
        if expired {
            let _ = emit_closed(&connection, id, CloseReason::Expired);
        }
    });
}

struct Notifications {
    registry: Shared,
    connection: Connection,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    fn get_capabilities(&self) -> Vec<String> {
        vec!["actions".to_owned(), "body".to_owned()]
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let resident = hints
            .get("resident")
            .and_then(|value| bool::try_from(value).ok())
            .unwrap_or(false);
        let (id, generation) = {
            let mut registry = lock(&self.registry);
            let id = match replaces_id {
                0 => registry.fresh_id(),
                id => id,
            };
            registry.generation += 1;
            let generation = registry.generation;
            let notification = Notification {
                id,
                app_name,
                app_icon,
                summary,
                body,
                actions: actions
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|[key, label]| (key.clone(), label.clone()))
                    .collect(),
                resident,
            };
            registry.active.insert(id, (notification, generation));
            (id, generation)
        };
        let timeout = match expire_timeout {
            0 => None,
            milliseconds if milliseconds > 0 => {
                Some(Duration::from_millis(milliseconds.unsigned_abs().into()))
            }
            _ => Some(DEFAULT_TIMEOUT),
        };
        if let Some(timeout) = timeout {
            schedule_expiry(
                self.connection.clone(),
                Arc::clone(&self.registry),
                id,
                generation,
                timeout,
            );
        }
        id
    }

    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        if lock(&self.registry).active.remove(&id).is_none() {
            return Err(zbus::fdo::Error::Failed(String::new()));
        }
        Self::notification_closed(&emitter, id, CloseReason::Closed as u32).await?;
        Ok(())
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "Unbound".to_owned(),
            "Unbound".to_owned(),
            env!("CARGO_PKG_VERSION").to_owned(),
            SPEC_VERSION.to_owned(),
        )
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}
