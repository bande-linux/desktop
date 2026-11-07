use std::collections::BTreeMap;
use std::sync::mpsc::{self, Sender};
use std::thread;

use bande_power::brightness::BrightnessProxyBlocking;
use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;
use zbus::proxy::CacheProperties;

#[derive(Debug, Clone, PartialEq)]
pub struct DisplayLevel {
    pub connector: String,
    pub name: String,
    pub level: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LevelRequest {
    Set(String, f32),
    Refresh,
}

fn proxy(session: &Connection) -> zbus::Result<BrightnessProxyBlocking<'static>> {
    BrightnessProxyBlocking::builder(session)
        .cache_properties(CacheProperties::No)
        .build()
}

fn levels(proxy: &BrightnessProxyBlocking<'_>) -> zbus::Result<Vec<DisplayLevel>> {
    Ok(proxy
        .displays()?
        .into_iter()
        .map(|(connector, name, level)| DisplayLevel {
            connector,
            name,
            level: level as f32,
        })
        .collect())
}

pub fn follow_levels(
    session: &Connection,
    mut changed: impl FnMut(Vec<DisplayLevel>) -> bool + Send + 'static,
) -> zbus::Result<()> {
    let proxy = proxy(session)?;
    let changes = PropertiesProxy::builder(session)
        .destination(proxy.inner().destination().to_owned())?
        .path(proxy.inner().path().to_owned())?
        .build()?
        .receive_properties_changed()?;
    thread::Builder::new()
        .name("brightness-follow".to_owned())
        .spawn(move || {
            let mut publish = move || match levels(&proxy) {
                Ok(levels) => changed(levels),
                Err(error) => {
                    tracing::warn!("cannot read the brightness: {error}");
                    true
                }
            };
            if publish() {
                changes.into_iter().all(|_| publish());
            }
        })?;
    Ok(())
}

pub fn level_requests(
    session: &Connection,
    failed: impl Fn(zbus::Error) + Send + 'static,
) -> zbus::Result<Sender<LevelRequest>> {
    let proxy = proxy(session)?;
    let (requests, received) = mpsc::channel();
    thread::Builder::new()
        .name("brightness".to_owned())
        .spawn(move || {
            while let Ok(first) = received.recv() {
                let mut wanted = BTreeMap::new();
                let mut refresh = false;
                for request in std::iter::once(first).chain(received.try_iter()) {
                    match request {
                        LevelRequest::Set(connector, level) => {
                            wanted.insert(connector, level);
                        }
                        LevelRequest::Refresh => refresh = true,
                    }
                }
                for (connector, level) in wanted {
                    if let Err(error) = proxy.set_level(&connector, f64::from(level)) {
                        failed(error);
                    }
                }
                if refresh && let Err(error) = proxy.refresh() {
                    failed(error);
                }
            }
        })?;
    Ok(requests)
}
