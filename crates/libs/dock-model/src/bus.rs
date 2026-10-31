use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};
use std::thread;

use unbound_bus::follow_properties;
use unbound_config::client::{ConfigProxy, INTERFACE, Properties};
use unbound_config::{BUS_NAME, BarEdge, OBJECT_PATH};
use unbound_inbox::Outbox;
use zbus::blocking::{Connection, MessageIterator};
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedValue;
use zbus::{MatchRule, message};

use crate::LauncherUpdate;

const LAUNCHER_INTERFACE: &str = "com.canonical.Unity.LauncherEntry";

#[derive(Debug, Clone, PartialEq)]
pub enum BusEvent {
    Apps(Vec<String>),
    Edge(BarEdge),
    AutoHide(bool),
    IconSize(u32),
    Names(bool),
    Launcher(String, LauncherUpdate),
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

fn follow_launcher_entries(session: Connection, outbox: Outbox<BusEvent>) {
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

fn config(session: &Connection) -> zbus::Result<ConfigProxy<'static>> {
    ConfigProxy::builder(session)
        .cache_properties(CacheProperties::No)
        .build()
}

fn bar_settings(changed: Properties) -> Vec<BusEvent> {
    let edge = changed.bar_edge.as_deref().and_then(BarEdge::from_keyword);
    [
        changed.bar_icon_size.map(BusEvent::IconSize),
        changed.bar_auto_hide.map(BusEvent::AutoHide),
        changed.bar_names.map(BusEvent::Names),
        edge.map(BusEvent::Edge),
        changed.bar_apps.map(BusEvent::Apps),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn follow_bar_settings(session: Connection, outbox: Outbox<BusEvent>) {
    spawn("bar-settings", move || {
        let mut known: Vec<BusEvent> = Vec::new();
        let followed = follow_properties(&session, BUS_NAME, OBJECT_PATH, INTERFACE, |changed| {
            for event in bar_settings(changed) {
                if known.contains(&event) {
                    continue;
                }
                known.retain(|old| std::mem::discriminant(old) != std::mem::discriminant(&event));
                known.push(event.clone());
                if !outbox.send(event) {
                    return false;
                }
            }
            true
        });
        if let Err(error) = followed {
            tracing::warn!("cannot follow the bar's settings in org.unbound.Config: {error}");
        }
    });
}

pub fn keep_writer(session: Connection) -> Sender<Vec<String>> {
    let (sender, receiver) = mpsc::channel::<Vec<String>>();
    spawn("bar-keep", move || {
        let config = match config(&session) {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!("cannot reach org.unbound.Config to keep apps: {error}");
                return;
            }
        };
        while let Ok(first) = receiver.recv() {
            let apps = receiver.try_iter().last().unwrap_or(first);
            if let Err(error) = config.set_bar_apps(&apps) {
                tracing::warn!("cannot change the apps kept in the bar: {error}");
            }
        }
    });
    sender
}

pub fn write_names(session: Connection, names: bool) {
    spawn("bar-names", move || {
        let written = config(&session).and_then(|config| config.set_bar_names(names));
        if let Err(error) = written {
            tracing::warn!("cannot save whether the bar shows names: {error}");
        }
    });
}

pub fn follow(session: &Connection, outbox: Outbox<BusEvent>) {
    follow_launcher_entries(session.clone(), outbox.clone());
    follow_bar_settings(session.clone(), outbox);
}
