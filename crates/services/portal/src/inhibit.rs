use std::collections::{HashMap, HashSet};
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bande_xdg::{Locale, app_name, applications, current_desktops, data_dirs};
use zbus::object_server::{ObjectServer, SignalEmitter};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use crate::OBJECT_PATH;
use crate::file_chooser::option;
use crate::request::{OTHER, SUCCESS};

pub const SUSPEND: u32 = 4;
pub const IDLE: u32 = 8;
pub const RUNNING: u32 = 1;

const SCREENSAVER: &str = "org.freedesktop.ScreenSaver";
const SCREENSAVER_PATH: &str = "/org/freedesktop/ScreenSaver";
const INTERFACE: &str = "org.freedesktop.impl.portal.Inhibit";

type Options = HashMap<String, OwnedValue>;
type Monitors = Arc<Mutex<HashSet<OwnedObjectPath>>>;

fn lock(monitors: &Monitors) -> MutexGuard<'_, HashSet<OwnedObjectPath>> {
    monitors.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone)]
pub struct Inhibit {
    session: zbus::Connection,
    system: Option<zbus::Connection>,
    locked: Arc<AtomicBool>,
    monitors: Monitors,
}

fn display_name(app_id: &str) -> String {
    let entries = applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    );
    app_name(&entries, app_id)
}

fn state(locked: bool) -> HashMap<&'static str, Value<'static>> {
    HashMap::from([
        ("screensaver-active", Value::from(locked)),
        ("session-state", Value::from(RUNNING)),
    ])
}

impl Inhibit {
    pub fn serve(
        session: &zbus::blocking::Connection,
        system: Option<&zbus::blocking::Connection>,
    ) -> zbus::Result<()> {
        let inhibit = Self {
            session: session.inner().clone(),
            system: system.map(|system| system.inner().clone()),
            locked: Arc::default(),
            monitors: Monitors::default(),
        };
        if let Some(system) = system {
            let (locked, monitors, session) = (
                Arc::clone(&inhibit.locked),
                Arc::clone(&inhibit.monitors),
                session.clone(),
            );
            let followed = bande_bus::logind::follow_locked_hint(system, move |now| {
                if locked.swap(now, Ordering::SeqCst) == now {
                    return;
                }
                let watching: Vec<OwnedObjectPath> = lock(&monitors).iter().cloned().collect();
                for monitor in watching {
                    let _ = session.emit_signal(
                        None::<&str>,
                        OBJECT_PATH,
                        INTERFACE,
                        "StateChanged",
                        &(monitor, state(now)),
                    );
                }
            });
            if let Err(error) = followed {
                tracing::warn!("cannot follow the screen lock: {error}");
            }
        }
        session.object_server().at(OBJECT_PATH, inhibit)?;
        Ok(())
    }

    async fn hold_idle(&self, app: &str, reason: &str) -> Option<u32> {
        let reply = self
            .session
            .call_method(
                Some(SCREENSAVER),
                SCREENSAVER_PATH,
                Some(SCREENSAVER),
                "Inhibit",
                &(app, reason),
            )
            .await
            .and_then(|reply| reply.body().deserialize::<u32>());
        reply
            .inspect_err(|error| tracing::warn!("cannot keep the screen awake for {app}: {error}"))
            .ok()
    }

    async fn hold_sleep(&self, app: &str, reason: &str) -> Option<OwnedFd> {
        let system = self.system.as_ref()?;
        let reply = system
            .call_method(
                Some("org.freedesktop.login1"),
                "/org/freedesktop/login1",
                Some("org.freedesktop.login1.Manager"),
                "Inhibit",
                &("sleep", app, reason, "block"),
            )
            .await
            .and_then(|reply| reply.body().deserialize::<zbus::zvariant::OwnedFd>());
        reply
            .map(OwnedFd::from)
            .inspect_err(|error| {
                tracing::warn!("cannot keep the computer awake for {app}: {error}")
            })
            .ok()
    }
}

struct Inhibition {
    session: zbus::Connection,
    path: OwnedObjectPath,
    cookie: Option<u32>,
    _sleep: Option<OwnedFd>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl Inhibition {
    async fn close(&self, #[zbus(object_server)] server: &ObjectServer) {
        if let Some(cookie) = self.cookie {
            let _ = self
                .session
                .call_method(
                    Some(SCREENSAVER),
                    SCREENSAVER_PATH,
                    Some(SCREENSAVER),
                    "UnInhibit",
                    &cookie,
                )
                .await;
        }
        let _ = server.remove::<Self, _>(&self.path).await;
    }
}

struct Monitor {
    monitors: Monitors,
    path: OwnedObjectPath,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl Monitor {
    async fn close(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        lock(&self.monitors).remove(&self.path);
        let _ = Self::closed(&emitter).await;
        let _ = server.remove::<Self, _>(&self.path).await;
    }

    #[zbus(signal)]
    async fn closed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Inhibit")]
impl Inhibit {
    async fn inhibit(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        _window: String,
        flags: u32,
        options: Options,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<()> {
        let reason: String = option(&options, "reason").unwrap_or_default();
        let app = blocking::unblock(move || display_name(&app_id)).await;
        let cookie = match flags & IDLE {
            0 => None,
            _ => self.hold_idle(&app, &reason).await,
        };
        let sleep = match flags & SUSPEND {
            0 => None,
            _ => self.hold_sleep(&app, &reason).await,
        };
        server
            .at(
                &handle,
                Inhibition {
                    session: self.session.clone(),
                    path: OwnedObjectPath::from(handle.to_owned()),
                    cookie,
                    _sleep: sleep,
                },
            )
            .await?;
        Ok(())
    }

    async fn create_monitor(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        _window: String,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> u32 {
        let path = OwnedObjectPath::from(session_handle.to_owned());
        let monitor = Monitor {
            monitors: Arc::clone(&self.monitors),
            path: path.clone(),
        };
        if server.at(&path, monitor).await.is_err() {
            return OTHER;
        }
        lock(&self.monitors).insert(path.clone());
        let locked = self.locked.load(Ordering::SeqCst);
        let after_reply = connection.clone();
        connection
            .executor()
            .spawn(
                async move {
                    let _ = after_reply
                        .emit_signal(
                            None::<&str>,
                            OBJECT_PATH,
                            INTERFACE,
                            "StateChanged",
                            &(path, state(locked)),
                        )
                        .await;
                },
                "inhibit monitor state",
            )
            .detach();
        SUCCESS
    }

    fn query_end_response(&self, _session_handle: ObjectPath<'_>) {}

    #[zbus(signal)]
    async fn state_changed(
        emitter: &SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        state: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<()>;
}
