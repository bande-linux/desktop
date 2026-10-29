use std::collections::HashMap;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::Sender;
use std::thread;

use unbound_config::{BUS_NAME, OBJECT_PATH};
use zbus::blocking::fdo::PropertiesProxy;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::zvariant::OwnedValue;
use zbus::{MatchRule, message};

use crate::LauncherUpdate;

const LAUNCHER_INTERFACE: &str = "com.canonical.Unity.LauncherEntry";
const CONFIG_INTERFACE: &str = "org.unbound.Config";

#[derive(Debug, Clone, PartialEq)]
pub enum BusEvent {
    DockApps(Vec<String>),
    AutoHide(bool),
    IconSize(u32),
    Launcher(String, LauncherUpdate),
}

#[derive(Clone)]
struct Outbox {
    events: Sender<BusEvent>,
    wake: std::sync::Arc<UnixStream>,
}

impl Outbox {
    fn send(&self, event: BusEvent) -> bool {
        if self.events.send(event).is_err() {
            return false;
        }
        let _ = (&*self.wake).write(&[1]);
        true
    }
}

pub fn app_id_from_uri(uri: &str) -> Option<String> {
    let name = uri.strip_prefix("application://").unwrap_or(uri);
    let app_id = name.strip_suffix(".desktop").unwrap_or(name);
    (!app_id.is_empty()).then(|| app_id.to_owned())
}

pub fn launcher_update(properties: &HashMap<String, OwnedValue>) -> LauncherUpdate {
    let int = |key: &str| {
        let value = properties.get(key)?;
        value
            .downcast_ref::<i64>()
            .ok()
            .or_else(|| value.downcast_ref::<i32>().ok().map(i64::from))
            .or_else(|| value.downcast_ref::<u32>().ok().map(i64::from))
    };
    let flag = |key: &str| properties.get(key)?.downcast_ref::<bool>().ok();
    LauncherUpdate {
        count: int("count"),
        count_visible: flag("count-visible"),
        progress: properties
            .get("progress")
            .and_then(|value| value.downcast_ref::<f64>().ok()),
        progress_visible: flag("progress-visible"),
        urgent: flag("urgent"),
    }
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = thread::Builder::new().name(name.to_owned()).spawn(work) {
        tracing::warn!("cannot start the {name} thread: {error}");
    }
}

fn follow_launcher_entries(session: Connection, outbox: Outbox) {
    spawn("launcher-entries", move || {
        let rule = MatchRule::builder()
            .msg_type(message::Type::Signal)
            .interface(LAUNCHER_INTERFACE)
            .and_then(|rule| rule.member("Update"))
            .map(|rule| rule.build());
        let iterator = rule.and_then(|rule| MessageIterator::for_match_rule(rule, &session, None));
        let iterator = match iterator {
            Ok(iterator) => iterator,
            Err(error) => {
                tracing::warn!("cannot follow app badges: {error}");
                return;
            }
        };
        for message in iterator.flatten() {
            let Ok((uri, properties)) = message
                .body()
                .deserialize::<(String, HashMap<String, OwnedValue>)>()
            else {
                continue;
            };
            let Some(app_id) = app_id_from_uri(&uri) else {
                continue;
            };
            if !outbox.send(BusEvent::Launcher(app_id, launcher_update(&properties))) {
                break;
            }
        }
    });
}

fn follow_dock_apps(session: Connection, outbox: Outbox) {
    spawn("dock-apps", move || {
        let follow = || -> zbus::Result<()> {
            let config = Proxy::new(&session, BUS_NAME, OBJECT_PATH, CONFIG_INTERFACE)?;
            let changes = PropertiesProxy::builder(&session)
                .destination(BUS_NAME)?
                .path(OBJECT_PATH)?
                .build()?
                .receive_properties_changed()?;
            let publish = || {
                if let Ok(size) = config.get_property::<u32>("DockIconSize")
                    && !outbox.send(BusEvent::IconSize(size))
                {
                    return Ok(false);
                }
                if let Ok(auto_hide) = config.get_property::<bool>("DockAutoHide")
                    && !outbox.send(BusEvent::AutoHide(auto_hide))
                {
                    return Ok(false);
                }
                config
                    .get_property::<Vec<String>>("DockApps")
                    .map(|apps| outbox.send(BusEvent::DockApps(apps)))
            };
            if !publish()? {
                return Ok(());
            }
            for _ in changes {
                if !publish()? {
                    break;
                }
            }
            Ok(())
        };
        if let Err(error) = follow() {
            tracing::warn!("cannot follow the Dock's apps in org.unbound.Config: {error}");
        }
    });
}

pub fn follow(events: Sender<BusEvent>, wake: UnixStream) {
    let session = match Connection::session() {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!("no session bus: {error}");
            return;
        }
    };
    let outbox = Outbox {
        events,
        wake: std::sync::Arc::new(wake),
    };
    follow_launcher_entries(session.clone(), outbox.clone());
    follow_dock_apps(session, outbox);
}
