use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use notify::event::{AccessKind, AccessMode, ModifyKind};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value};

use crate::{Appearance, Config, ConfigStore, PanelEdge, ReloadError};

pub const BUS_NAME: &str = "org.unbound.Config";
pub const OBJECT_PATH: &str = "/org/unbound/Config";

#[derive(Debug)]
pub enum ServiceError {
    Bus(zbus::Error),
    Watch(notify::Error),
    Path(PathBuf),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bus(error) => write!(f, "D-Bus: {error}"),
            Self::Watch(error) => write!(f, "cannot watch the configuration: {error}"),
            Self::Path(path) => write!(f, "{} has no parent directory", path.display()),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<zbus::Error> for ServiceError {
    fn from(error: zbus::Error) -> Self {
        Self::Bus(error)
    }
}

impl From<notify::Error> for ServiceError {
    fn from(error: notify::Error) -> Self {
        Self::Watch(error)
    }
}

pub struct ConfigService {
    _watcher: RecommendedWatcher,
    _connection: Connection,
}

impl ConfigService {
    pub fn start(
        connection: &Connection,
        path: &Path,
        store: Arc<ConfigStore>,
        on_error: impl Fn(ReloadError) + Send + 'static,
    ) -> Result<Self, ServiceError> {
        let directory = path
            .parent()
            .ok_or_else(|| ServiceError::Path(path.to_path_buf()))?;
        let file_name: OsString = path
            .file_name()
            .ok_or_else(|| ServiceError::Path(path.to_path_buf()))?
            .to_owned();
        std::fs::create_dir_all(directory).map_err(|error| ServiceError::Watch(error.into()))?;

        if path.exists()
            && let Err(error) = store.reload(path)
        {
            on_error(error);
        }

        connection.object_server().at(
            OBJECT_PATH,
            ConfigInterface {
                store: Arc::clone(&store),
            },
        )?;
        connection.request_name(BUS_NAME)?;
        let interface = connection
            .object_server()
            .interface::<_, ConfigInterface>(OBJECT_PATH)?;

        let watched = path.to_path_buf();
        let mut published = store.current();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let Ok(event) = event else { return };
                let relevant = matches!(
                    event.kind,
                    EventKind::Access(AccessKind::Close(AccessMode::Write))
                        | EventKind::Modify(ModifyKind::Name(_))
                ) && event
                    .paths
                    .iter()
                    .any(|changed| changed.file_name() == Some(file_name.as_os_str()));
                if !relevant {
                    return;
                }
                match store.reload(&watched) {
                    Ok(config) => {
                        publish_changes(&interface, &published, &config);
                        published = config;
                    }
                    Err(error) => on_error(error),
                }
            })?;
        watcher.watch(directory, RecursiveMode::NonRecursive)?;
        Ok(Self {
            _watcher: watcher,
            _connection: connection.clone(),
        })
    }
}

fn publish_changes(
    interface: &zbus::blocking::object_server::InterfaceRef<ConfigInterface>,
    before: &Config,
    after: &Config,
) {
    let emitter = interface.signal_emitter();
    let guard = interface.get();
    zbus::block_on(async {
        if before.appearance != after.appearance {
            let _ = guard.appearance_changed(emitter).await;
        }
        if before.panel_edge != after.panel_edge {
            let _ = guard.panel_edge_changed(emitter).await;
        }
        if before.idle.dim_after != after.idle.dim_after {
            let _ = guard.idle_dim_after_changed(emitter).await;
        }
        if before.idle.lock_after != after.idle.lock_after {
            let _ = guard.idle_lock_after_changed(emitter).await;
        }
        if before.outputs != after.outputs {
            let _ = guard.outputs_changed(emitter).await;
        }
    });
}

struct ConfigInterface {
    store: Arc<ConfigStore>,
}

#[zbus::interface(name = "org.unbound.Config")]
impl ConfigInterface {
    #[zbus(property)]
    fn appearance(&self) -> String {
        match self.store.current().appearance {
            Appearance::Dark => "dark",
            Appearance::Light => "light",
            Appearance::Automatic => "automatic",
        }
        .to_owned()
    }

    #[zbus(property)]
    fn panel_edge(&self) -> String {
        match self.store.current().panel_edge {
            PanelEdge::Left => "left",
            PanelEdge::Bottom => "bottom",
            PanelEdge::Right => "right",
            PanelEdge::Top => "top",
        }
        .to_owned()
    }

    #[zbus(property)]
    fn idle_dim_after(&self) -> u64 {
        self.store.current().idle.dim_after.as_secs()
    }

    #[zbus(property)]
    fn idle_lock_after(&self) -> u64 {
        self.store.current().idle.lock_after.as_secs()
    }

    #[zbus(property)]
    fn outputs(&self) -> HashMap<String, HashMap<String, OwnedValue>> {
        let owned = |value: Value<'_>| OwnedValue::try_from(value).ok();
        self.store
            .current()
            .outputs
            .iter()
            .map(|(connector, output)| {
                let settings = [
                    ("scale", output.scale.and_then(|scale| owned(scale.into()))),
                    (
                        "refresh-rate",
                        output.refresh_rate.and_then(|rate| owned(rate.into())),
                    ),
                    (
                        "variable-refresh-rate",
                        output
                            .variable_refresh_rate
                            .and_then(|enabled| owned(enabled.into())),
                    ),
                ]
                .into_iter()
                .filter_map(|(key, value)| Some((key.to_owned(), value?)))
                .collect();
                (connector.clone(), settings)
            })
            .collect()
    }
}
