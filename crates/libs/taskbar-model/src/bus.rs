use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};
use std::thread;

use bande_config::{BUS_NAME, OBJECT_PATH};
use bande_inbox::Outbox;
use zbus::blocking::fdo::PropertiesProxy;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::zvariant::{OwnedValue, Value};
use zbus::{MatchRule, message};

use crate::LauncherUpdate;

const LAUNCHER_INTERFACE: &str = "com.canonical.Unity.LauncherEntry";
const CONFIG_INTERFACE: &str = "org.bande.Config";

#[derive(Debug, Clone, PartialEq)]
pub enum BusEvent {
    Apps(Vec<String>),
    AutoHide(bool),
    Form { floating: bool, gap: u32 },
    IconSize(u32),
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
            .and_then(|value| value.downcast_ref::<f64>().ok())
            .filter(|progress| progress.is_finite()),
        progress_visible: flag("progress-visible"),
        urgent: flag("urgent"),
    }
}

/// Shows `count` on `app_id`'s Taskbar button, or no badge for 0, and with `urgent` also asks
/// for attention while the count is above 0.
pub fn show_badge(
    connection: &Connection,
    app_id: &str,
    count: u32,
    urgent: bool,
) -> zbus::Result<()> {
    let path: String = app_id
        .split('.')
        .flat_map(|part| {
            std::iter::once('/').chain(
                part.chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }),
            )
        })
        .collect();
    let properties = HashMap::from([
        ("count", Value::from(i64::from(count))),
        ("count-visible", Value::from(count > 0)),
        ("urgent", Value::from(urgent && count > 0)),
    ]);
    let uri = format!("application://{app_id}.desktop");
    connection.emit_signal(
        None::<&str>,
        path.as_str(),
        LAUNCHER_INTERFACE,
        "Update",
        &(uri.as_str(), properties),
    )
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

fn bar_settings(config: &Proxy<'_>) -> zbus::Result<Vec<BusEvent>> {
    let mut events = Vec::new();
    if let Ok(size) = config.get_property::<u32>("BarIconSize") {
        events.push(BusEvent::IconSize(size));
    }
    if let Ok(auto_hide) = config.get_property::<bool>("BarAutoHide") {
        events.push(BusEvent::AutoHide(auto_hide));
    }
    if let (Ok(floating), Ok(gap)) = (
        config.get_property::<bool>("BarFloating"),
        config.get_property::<u32>("BarGap"),
    ) {
        events.push(BusEvent::Form { floating, gap });
    }
    events.push(BusEvent::Apps(
        config.get_property::<Vec<String>>("BarApps")?,
    ));
    Ok(events)
}

fn follow_bar_settings(session: Connection, outbox: Outbox<BusEvent>) {
    spawn("bar-settings", move || {
        let follow = || -> zbus::Result<()> {
            let config = zbus::blocking::proxy::Builder::<Proxy>::new(&session)
                .destination(BUS_NAME)?
                .path(OBJECT_PATH)?
                .interface(CONFIG_INTERFACE)?
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()?;
            let changes = PropertiesProxy::builder(&session)
                .destination(BUS_NAME)?
                .path(OBJECT_PATH)?
                .build()?
                .receive_properties_changed()?;
            let mut known: Vec<BusEvent> = Vec::new();
            let mut publish = || -> zbus::Result<bool> {
                for event in bar_settings(&config)? {
                    if known.contains(&event) {
                        continue;
                    }
                    known.retain(|old| {
                        std::mem::discriminant(old) != std::mem::discriminant(&event)
                    });
                    known.push(event.clone());
                    if !outbox.send(event) {
                        return Ok(false);
                    }
                }
                Ok(true)
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
            tracing::warn!("cannot follow the bar's settings in org.bande.Config: {error}");
        }
    });
}

pub fn keep_writer(session: Connection) -> Sender<Vec<String>> {
    let (sender, receiver) = mpsc::channel::<Vec<String>>();
    spawn("bar-keep", move || {
        let config = Proxy::new(&session, BUS_NAME, OBJECT_PATH, CONFIG_INTERFACE);
        let config = match config {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!("cannot reach org.bande.Config to keep apps: {error}");
                return;
            }
        };
        while let Ok(first) = receiver.recv() {
            let apps = receiver.try_iter().last().unwrap_or(first);
            if let Err(error) = config.set_property("BarApps", apps) {
                tracing::warn!("cannot change the apps kept in the bar: {error}");
            }
        }
    });
    sender
}

pub fn follow(session: &Connection, outbox: Outbox<BusEvent>) {
    follow_launcher_entries(session.clone(), outbox.clone());
    follow_bar_settings(session.clone(), outbox);
}
