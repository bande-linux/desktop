use std::collections::HashMap;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ClientId;
use zbus::blocking::Connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

use crate::shell::{is_shell, toplevel_names};
use crate::state::{Compositor, State, client_pid};

const APPS_NAME: &str = "org.bande.Apps";
const PATH: &str = "/org/bande/Apps";
const SEAT: &str = "seat0";

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunningApp {
    app_id: String,
    sandboxed: Option<String>,
    active: bool,
}

type Shown = Arc<Mutex<Vec<RunningApp>>>;

fn lock(shown: &Shown) -> MutexGuard<'_, Vec<RunningApp>> {
    shown.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
pub(crate) struct AppStates {
    published: Option<AppStatesHandle>,
    last: Vec<RunningApp>,
    sandboxes: HashMap<ClientId, Option<String>>,
}

pub struct AppStatesHandle {
    shown: Shown,
    changed: Sender<()>,
}

fn sandboxed_app_id(flatpak_info: &str) -> Option<String> {
    let mut application = false;
    for line in flatpak_info.lines().map(str::trim) {
        if line.starts_with('[') {
            application = line == "[Application]";
        } else if application && let Some(name) = line.strip_prefix("name=") {
            return Some(name.trim().to_owned());
        }
    }
    None
}

fn sandbox_of(pid: i32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/root/.flatpak-info"))
        .ok()
        .as_deref()
        .and_then(sandboxed_app_id)
}

struct Apps {
    shown: Shown,
}

#[zbus::interface(name = "org.bande.Apps")]
impl Apps {
    fn get_running_applications(&self) -> HashMap<String, HashMap<String, OwnedValue>> {
        let mut running: HashMap<String, HashMap<String, OwnedValue>> = HashMap::new();
        for app in lock(&self.shown).iter() {
            let details = running.entry(app.app_id.clone()).or_default();
            let entries = [
                ("sandboxed-app-id", app.sandboxed.clone().map(Value::from)),
                (
                    "active-on-seats",
                    app.active.then(|| Value::from(vec![SEAT])),
                ),
            ];
            for (key, value) in entries {
                if let Some(value) = value.and_then(|value| OwnedValue::try_from(value).ok()) {
                    details.insert(key.to_owned(), value);
                }
            }
        }
        running
    }

    #[zbus(signal)]
    async fn running_applications_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

pub fn serve_app_states(connection: &Connection) -> zbus::Result<AppStatesHandle> {
    let shown = Shown::default();
    connection.object_server().at(
        PATH,
        Apps {
            shown: Arc::clone(&shown),
        },
    )?;
    connection.request_name(APPS_NAME)?;
    let (changed, received) = channel::<()>();
    let emitting = connection.clone();
    thread::spawn(move || {
        for () in received {
            let _ = emitting.emit_signal(
                None::<&str>,
                PATH,
                APPS_NAME,
                "RunningApplicationsChanged",
                &(),
            );
        }
    });
    Ok(AppStatesHandle { shown, changed })
}

impl Compositor {
    pub fn publish_app_states(&mut self, handle: AppStatesHandle) {
        self.state.app_states.published = Some(handle);
    }
}

impl State {
    pub(crate) fn note_app_states(&mut self) {
        if self.app_states.published.is_none() {
            return;
        }
        let mut running: Vec<RunningApp> = Vec::new();
        let mut present: Vec<ClientId> = Vec::new();
        for window in self.windows.iter().filter(|window| window.size.is_some()) {
            let surface = window.toplevel.wl_surface();
            let Some(client) = surface.client().filter(|client| !is_shell(client)) else {
                continue;
            };
            let id = client.id();
            let sandboxed = self
                .app_states
                .sandboxes
                .entry(id.clone())
                .or_insert_with(|| client_pid(&client).and_then(sandbox_of))
                .clone();
            present.push(id);
            let app_id = toplevel_names(surface).1;
            if app_id.is_empty() && sandboxed.is_none() {
                continue;
            }
            let active = !app_id.is_empty() && app_id == self.active_app;
            match running.iter_mut().find(|known| known.app_id == app_id) {
                Some(known) => known.active |= active,
                None => running.push(RunningApp {
                    app_id,
                    sandboxed,
                    active,
                }),
            }
        }
        self.app_states
            .sandboxes
            .retain(|id, _| present.contains(id));
        if running == self.app_states.last {
            return;
        }
        if let Some(published) = &self.app_states.published {
            *lock(&published.shown) = running.clone();
            let _ = published.changed.send(());
        }
        self.app_states.last = running;
    }
}
