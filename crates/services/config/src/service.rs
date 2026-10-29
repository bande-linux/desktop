use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use notify::event::{AccessKind, AccessMode, ModifyKind};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use zbus::blocking::Connection;
use zbus::fdo::Error::{Failed, InvalidArgs};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

use crate::{
    Appearance, Config, ConfigStore, DockEdge, Output, ReloadError, Setting, WallpaperVariant,
};

pub const BUS_NAME: &str = "org.unbound.Config";
pub const OBJECT_PATH: &str = "/org/unbound/Config";

const STAGING_NAME: &str = ".config.kdl.new";

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

type Published = Arc<Mutex<Arc<Config>>>;

fn lock(published: &Published) -> MutexGuard<'_, Arc<Config>> {
    published.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ConfigService {
    pub fn start(
        connection: &Connection,
        path: &Path,
        store: Arc<ConfigStore>,
        on_reload: impl Fn(Result<(), ReloadError>) + Send + 'static,
    ) -> Result<Self, ServiceError> {
        let directory = path
            .parent()
            .ok_or_else(|| ServiceError::Path(path.to_path_buf()))?;
        let file_name: OsString = path
            .file_name()
            .ok_or_else(|| ServiceError::Path(path.to_path_buf()))?
            .to_owned();
        std::fs::create_dir_all(directory).map_err(|error| ServiceError::Watch(error.into()))?;

        if path.exists() {
            on_reload(store.reload(path).map(drop));
        }
        let published = Arc::new(Mutex::new(store.current()));

        connection.object_server().at(
            OBJECT_PATH,
            ConfigInterface {
                store: Arc::clone(&store),
                path: path.to_path_buf(),
                published: Arc::clone(&published),
            },
        )?;
        connection.request_name(BUS_NAME)?;
        let interface = connection
            .object_server()
            .interface::<_, ConfigInterface>(OBJECT_PATH)?;

        let watched = path.to_path_buf();
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
                let mut published = lock(&published);
                match store.reload(&watched) {
                    Ok(config) => {
                        publish_changes(&interface, &published, &config);
                        *published = config;
                        on_reload(Ok(()));
                    }
                    Err(error) => on_reload(Err(error)),
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
        if before.dock_edge != after.dock_edge {
            let _ = guard.dock_edge_changed(emitter).await;
        }
        if before.idle.dim_after != after.idle.dim_after {
            let _ = guard.idle_dim_after_changed(emitter).await;
        }
        if before.idle.lock_after != after.idle.lock_after {
            let _ = guard.idle_lock_after_changed(emitter).await;
        }
        if before.idle.suspend_after() != after.idle.suspend_after() {
            let _ = guard.idle_suspend_after_changed(emitter).await;
        }
        if before.keyboard.repeat_rate != after.keyboard.repeat_rate {
            let _ = guard.key_repeat_rate_changed(emitter).await;
        }
        if before.keyboard.repeat_delay != after.keyboard.repeat_delay {
            let _ = guard.key_repeat_delay_changed(emitter).await;
        }
        if before.wallpaper.light != after.wallpaper.light {
            let _ = guard.wallpaper_light_changed(emitter).await;
        }
        if before.wallpaper.dark != after.wallpaper.dark {
            let _ = guard.wallpaper_dark_changed(emitter).await;
        }
        if before.outputs != after.outputs {
            let _ = guard.outputs_changed(emitter).await;
        }
    });
}

fn keyword<T: Copy>(
    choices: &[T],
    name: impl Fn(T) -> &'static str,
    value: &str,
) -> zbus::fdo::Result<T> {
    choices
        .iter()
        .copied()
        .find(|choice| name(*choice) == value)
        .ok_or_else(|| {
            let names: Vec<_> = choices
                .iter()
                .map(|choice| format!("\"{}\"", name(*choice)))
                .collect();
            InvalidArgs(format!("expected one of {}", names.join(", ")))
        })
}

fn output_settings(settings: &HashMap<String, OwnedValue>) -> zbus::fdo::Result<Output> {
    let mut output = Output::default();
    for (key, value) in settings {
        let mismatch = || InvalidArgs(format!("`{key}` has the wrong type"));
        match key.as_str() {
            "scale" => output.scale = Some(f64::try_from(value).map_err(|_| mismatch())?),
            "refresh-rate" => {
                output.refresh_rate = Some(f64::try_from(value).map_err(|_| mismatch())?)
            }
            "variable-refresh-rate" => {
                output.variable_refresh_rate = Some(bool::try_from(value).map_err(|_| mismatch())?)
            }
            _ => return Err(InvalidArgs(format!("unknown output setting `{key}`"))),
        }
    }
    Ok(output)
}

struct ConfigInterface {
    store: Arc<ConfigStore>,
    path: PathBuf,
    published: Published,
}

impl ConfigInterface {
    fn wallpaper(&self, variant: WallpaperVariant) -> String {
        self.store
            .current()
            .wallpaper
            .path(variant)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn set_wallpaper(&self, variant: WallpaperVariant, path: String) -> zbus::fdo::Result<()> {
        let path = (!path.is_empty()).then(|| PathBuf::from(path));
        self.write(&Setting::Wallpaper(variant, path))
    }

    fn write(&self, setting: &Setting) -> zbus::fdo::Result<()> {
        let mut published = lock(&self.published);
        let source = match std::fs::read_to_string(&self.path) {
            Ok(source) => source,
            Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(Failed(format!("cannot read the configuration: {error}")));
            }
        };
        if let Err(error) = Config::parse(&source) {
            return Err(Failed(format!(
                "the configuration is invalid at {error} and must be fixed first"
            )));
        }
        let edited = setting
            .apply(&source)
            .map_err(|error| InvalidArgs(error.message))?;
        let staging = self.path.with_file_name(STAGING_NAME);
        std::fs::write(&staging, &edited)
            .and_then(|()| std::fs::rename(&staging, &self.path))
            .map_err(|error| Failed(format!("cannot write the configuration: {error}")))?;
        *published = self
            .store
            .apply(&edited)
            .map_err(|error| Failed(error.to_string()))?;
        Ok(())
    }
}

#[zbus::interface(name = "org.unbound.Config")]
impl ConfigInterface {
    #[zbus(property)]
    fn appearance(&self) -> String {
        self.store.current().appearance.keyword().to_owned()
    }

    #[zbus(property)]
    fn set_appearance(&self, value: String) -> zbus::fdo::Result<()> {
        let appearance = keyword(
            &[Appearance::Dark, Appearance::Light, Appearance::Automatic],
            Appearance::keyword,
            &value,
        )?;
        self.write(&Setting::Appearance(appearance))
    }

    #[zbus(property)]
    fn dock_edge(&self) -> String {
        self.store.current().dock_edge.keyword().to_owned()
    }

    #[zbus(property)]
    fn set_dock_edge(&self, value: String) -> zbus::fdo::Result<()> {
        let edge = keyword(
            &[
                DockEdge::Left,
                DockEdge::Bottom,
                DockEdge::Right,
                DockEdge::Top,
            ],
            DockEdge::keyword,
            &value,
        )?;
        self.write(&Setting::DockEdge(edge))
    }

    #[zbus(property)]
    fn idle_dim_after(&self) -> u64 {
        self.store.current().idle.dim_after.as_secs()
    }

    #[zbus(property)]
    fn set_idle_dim_after(&self, seconds: u64) -> zbus::fdo::Result<()> {
        self.write(&Setting::IdleDimAfter(Duration::from_secs(seconds)))
    }

    #[zbus(property)]
    fn idle_lock_after(&self) -> u64 {
        self.store.current().idle.lock_after.as_secs()
    }

    #[zbus(property)]
    async fn set_idle_lock_after(
        &self,
        seconds: u64,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let before = self.store.current().idle.suspend_after();
        self.write(&Setting::IdleLockAfter(Duration::from_secs(seconds)))?;
        if before != self.store.current().idle.suspend_after() {
            self.idle_suspend_after_changed(&emitter).await?;
        }
        Ok(())
    }

    #[zbus(property)]
    fn idle_suspend_after(&self) -> u64 {
        self.store
            .current()
            .idle
            .suspend_after()
            .map_or(0, |after| after.as_secs())
    }

    #[zbus(property)]
    fn set_idle_suspend_after(&self, seconds: u64) -> zbus::fdo::Result<()> {
        let after = (seconds > 0).then(|| Duration::from_secs(seconds));
        self.write(&Setting::IdleSuspendAfter(after))
    }

    #[zbus(property)]
    fn key_repeat_rate(&self) -> u32 {
        self.store.current().keyboard.repeat_rate
    }

    #[zbus(property)]
    fn set_key_repeat_rate(&self, rate: u32) -> zbus::fdo::Result<()> {
        self.write(&Setting::KeyRepeatRate(rate))
    }

    #[zbus(property)]
    fn key_repeat_delay(&self) -> u32 {
        u32::try_from(self.store.current().keyboard.repeat_delay.as_millis()).unwrap_or(u32::MAX)
    }

    #[zbus(property)]
    fn set_key_repeat_delay(&self, milliseconds: u32) -> zbus::fdo::Result<()> {
        self.write(&Setting::KeyRepeatDelay(Duration::from_millis(
            milliseconds.into(),
        )))
    }

    #[zbus(property)]
    fn wallpaper_light(&self) -> String {
        self.wallpaper(WallpaperVariant::Light)
    }

    #[zbus(property)]
    fn set_wallpaper_light(&self, path: String) -> zbus::fdo::Result<()> {
        self.set_wallpaper(WallpaperVariant::Light, path)
    }

    #[zbus(property)]
    fn wallpaper_dark(&self) -> String {
        self.wallpaper(WallpaperVariant::Dark)
    }

    #[zbus(property)]
    fn set_wallpaper_dark(&self, path: String) -> zbus::fdo::Result<()> {
        self.set_wallpaper(WallpaperVariant::Dark, path)
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

    async fn set_output(
        &self,
        connector: String,
        settings: HashMap<String, OwnedValue>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let before = self.store.current();
        self.write(&Setting::Output(connector, output_settings(&settings)?))?;
        if before.outputs != self.store.current().outputs {
            self.outputs_changed(&emitter).await?;
        }
        Ok(())
    }
}
