mod center;
pub mod client;

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Instant;

use zbus::blocking::Connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedValue;

pub use center::{
    BANNER_DURATION, Center, CloseReason, Invoked, Notification, Presentation, Stack, Timeout,
    Urgency,
};

pub const BUS_NAME: &str = "org.freedesktop.Notifications";
pub const OBJECT_PATH: &str = "/org/freedesktop/Notifications";
pub const SPEC_VERSION: &str = "1.2";

type Presenter = Box<dyn FnMut(&Presentation) + Send>;

struct State {
    center: Center,
    presented: Presentation,
    presenter: Presenter,
    stopped: bool,
}

impl State {
    fn present(&mut self) {
        let presentation = self.center.presentation();
        if presentation != self.presented {
            (self.presenter)(&presentation);
            self.presented = presentation;
        }
    }
}

struct Shared {
    state: Mutex<State>,
    deadline_changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn update<T>(&self, change: impl FnOnce(&mut Center) -> T) -> T {
        let mut state = self.lock();
        let result = change(&mut state.center);
        state.present();
        self.deadline_changed.notify_all();
        result
    }
}

pub struct NotificationServer {
    connection: Connection,
    shared: Arc<Shared>,
}

impl NotificationServer {
    pub fn start(connection: &Connection) -> zbus::Result<Self> {
        Self::start_presenting(connection, |_| {})
    }

    pub fn start_presenting(
        connection: &Connection,
        presenter: impl FnMut(&Presentation) + Send + 'static,
    ) -> zbus::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                center: Center::default(),
                presented: Presentation::default(),
                presenter: Box::new(presenter),
                stopped: false,
            }),
            deadline_changed: Condvar::new(),
        });
        connection.object_server().at(
            OBJECT_PATH,
            Notifications {
                shared: Arc::clone(&shared),
            },
        )?;
        connection.object_server().at(
            OBJECT_PATH,
            NotificationCenter {
                shared: Arc::clone(&shared),
            },
        )?;
        connection.request_name(BUS_NAME)?;
        let timer = Arc::clone(&shared);
        let timer_connection = connection.clone();
        thread::spawn(move || expire(&timer, &timer_connection));
        Ok(Self {
            connection: connection.clone(),
            shared,
        })
    }

    pub fn active(&self) -> Vec<Notification> {
        self.shared.lock().center.notifications().cloned().collect()
    }

    pub fn dismiss(&self, id: u32) -> zbus::Result<bool> {
        let closed = self.shared.update(|center| center.close(id));
        if closed {
            emit_closed(&self.connection, id, CloseReason::Dismissed)?;
        }
        Ok(closed)
    }

    pub fn clear(&self, group: &str) -> zbus::Result<()> {
        for id in self.shared.update(|center| center.clear(group)) {
            emit_closed(&self.connection, id, CloseReason::Dismissed)?;
        }
        Ok(())
    }

    pub fn invoke_action(&self, id: u32, action_key: &str) -> zbus::Result<bool> {
        let Some(Invoked { closed }) = self.shared.update(|center| center.invoke(id, action_key))
        else {
            return Ok(false);
        };
        let emitter = SignalEmitter::new(self.connection.inner(), OBJECT_PATH)?;
        zbus::block_on(Notifications::action_invoked(&emitter, id, action_key))?;
        if closed {
            emit_closed(&self.connection, id, CloseReason::Dismissed)?;
        }
        Ok(true)
    }

    pub fn set_do_not_disturb(&self, enabled: bool) -> zbus::Result<()> {
        self.shared
            .update(|center| center.set_do_not_disturb(enabled));
        let interface = self
            .connection
            .object_server()
            .interface::<_, NotificationCenter>(OBJECT_PATH)?;
        zbus::block_on(
            interface
                .get()
                .do_not_disturb_changed(interface.signal_emitter()),
        )
    }
}

impl Drop for NotificationServer {
    fn drop(&mut self) {
        self.shared.lock().stopped = true;
        self.shared.deadline_changed.notify_all();
    }
}

fn expire(shared: &Shared, connection: &Connection) {
    let mut state = shared.lock();
    loop {
        if state.stopped {
            return;
        }
        let expired = state.center.advance(Instant::now());
        state.present();
        if !expired.is_empty() {
            drop(state);
            for id in expired {
                let _ = emit_closed(connection, id, CloseReason::Expired);
            }
            state = shared.lock();
            continue;
        }
        state = match state.center.next_deadline() {
            Some(deadline) => {
                let wait = deadline.saturating_duration_since(Instant::now());
                shared
                    .deadline_changed
                    .wait_timeout(state, wait)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
            None => shared
                .deadline_changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner),
        };
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

fn hint<'a, T: TryFrom<&'a OwnedValue>>(
    hints: &'a HashMap<String, OwnedValue>,
    name: &str,
) -> Option<T> {
    hints.get(name).and_then(|value| T::try_from(value).ok())
}

struct Notifications {
    shared: Arc<Shared>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    fn get_capabilities(&self) -> Vec<String> {
        ["actions", "body", "persistence"]
            .map(str::to_owned)
            .to_vec()
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
        let urgency = match hint::<u8>(&hints, "urgency") {
            Some(0) => Urgency::Low,
            Some(2) => Urgency::Critical,
            _ => Urgency::Normal,
        };
        let notification = Notification {
            id: replaces_id,
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
            urgency,
            desktop_entry: hint::<&str>(&hints, "desktop-entry")
                .filter(|entry| !entry.is_empty())
                .map(str::to_owned),
            resident: hint(&hints, "resident").unwrap_or(false),
            transient: hint(&hints, "transient").unwrap_or(false),
        };
        self.shared.update(|center| {
            center.post(
                notification,
                Timeout::from_milliseconds(expire_timeout),
                Instant::now(),
            )
        })
    }

    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        if !self.shared.update(|center| center.close(id)) {
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

struct NotificationCenter {
    shared: Arc<Shared>,
}

#[zbus::interface(name = "org.unbound.NotificationCenter")]
impl NotificationCenter {
    #[zbus(property)]
    fn do_not_disturb(&self) -> bool {
        self.shared.lock().center.do_not_disturb()
    }

    #[zbus(property)]
    fn set_do_not_disturb(&mut self, enabled: bool) {
        self.shared
            .update(|center| center.set_do_not_disturb(enabled));
    }
}
