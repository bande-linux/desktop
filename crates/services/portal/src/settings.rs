use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use unbound_graphics::{Palette, Scheme};
use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;
use zbus::object_server::SignalEmitter;
use zbus::proxy::CacheProperties;
use zbus::zvariant::{OwnedValue, Value};

use crate::OBJECT_PATH;

pub const APPEARANCE: &str = "org.freedesktop.appearance";
pub const GNOME_INTERFACE: &str = "org.gnome.desktop.interface";
pub const VERSION: u32 = 2;

const CONFIG_BUS_NAME: &str = "org.unbound.Config";
const CONFIG_PATH: &str = "/org/unbound/Config";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorScheme {
    NoPreference = 0,
    PreferDark = 1,
    PreferLight = 2,
}

impl ColorScheme {
    pub fn from_appearance(appearance: &str) -> Self {
        match appearance {
            "dark" => Self::PreferDark,
            "light" => Self::PreferLight,
            _ => Self::NoPreference,
        }
    }

    pub fn accent_color(self) -> (f64, f64, f64) {
        let scheme = match self {
            Self::PreferDark => Scheme::Dark,
            Self::PreferLight | Self::NoPreference => Scheme::Light,
        };
        let accent = Palette::for_scheme(scheme).accent_color;
        let channel = |value: u8| f64::from(value) / 255.0;
        (channel(accent.r), channel(accent.g), channel(accent.b))
    }

    fn gnome_color_scheme(self) -> &'static str {
        match self {
            Self::PreferDark => "prefer-dark",
            Self::PreferLight => "prefer-light",
            Self::NoPreference => "default",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorSettings {
    pub theme: String,
    pub size: i32,
}

impl CursorSettings {
    pub fn from_environment() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        Self {
            theme: var("XCURSOR_THEME").unwrap_or_else(|| "default".to_owned()),
            size: var("XCURSOR_SIZE")
                .and_then(|size| size.parse().ok())
                .filter(|size| *size > 0)
                .unwrap_or(24),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Look {
    scheme: ColorScheme,
    reduce_motion: bool,
}

impl Look {
    fn read(config: &ConfigProxyBlocking<'_>) -> Self {
        Self {
            scheme: ColorScheme::from_appearance(&config.appearance().unwrap_or_default()),
            reduce_motion: config.reduce_motion().unwrap_or(false),
        }
    }
}

type Namespaces = Vec<(&'static str, Vec<(&'static str, OwnedValue)>)>;

fn namespaces(look: Look, cursor: &CursorSettings) -> Namespaces {
    let Look {
        scheme,
        reduce_motion,
    } = look;
    let owned = |values: Vec<(&'static str, Value<'_>)>| {
        values
            .into_iter()
            .filter_map(|(key, value)| Some((key, OwnedValue::try_from(value).ok()?)))
            .collect()
    };
    vec![
        (
            APPEARANCE,
            owned(vec![
                ("color-scheme", Value::from(scheme as u32)),
                ("accent-color", Value::from(scheme.accent_color())),
                ("contrast", Value::from(0u32)),
                ("reduced-motion", Value::from(u32::from(reduce_motion))),
            ]),
        ),
        (
            GNOME_INTERFACE,
            owned(vec![
                ("color-scheme", Value::from(scheme.gnome_color_scheme())),
                ("cursor-theme", Value::from(cursor.theme.as_str())),
                ("cursor-size", Value::from(cursor.size)),
            ]),
        ),
    ]
}

fn matches(namespace: &str, filters: &[String]) -> bool {
    filters.is_empty()
        || filters.iter().any(|filter| match filter.strip_suffix('*') {
            Some(prefix) => namespace.starts_with(prefix),
            None => filter == namespace,
        })
}

#[zbus::proxy(
    interface = "org.unbound.Config",
    default_service = "org.unbound.Config",
    default_path = "/org/unbound/Config"
)]
trait Config {
    #[zbus(property)]
    fn appearance(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn reduce_motion(&self) -> zbus::Result<bool>;
}

pub struct Settings {
    look: Arc<Mutex<Look>>,
    cursor: CursorSettings,
}

impl Settings {
    pub fn serve(connection: &Connection, cursor: CursorSettings) -> zbus::Result<()> {
        let config = ConfigProxyBlocking::builder(connection)
            .cache_properties(CacheProperties::No)
            .build()?;
        let changes = PropertiesProxy::builder(connection)
            .destination(CONFIG_BUS_NAME)?
            .path(CONFIG_PATH)?
            .build()?
            .receive_properties_changed()?;
        let look = Arc::new(Mutex::new(Look::read(&config)));
        connection.object_server().at(
            OBJECT_PATH,
            Settings {
                look: Arc::clone(&look),
                cursor: cursor.clone(),
            },
        )?;
        let connection = connection.clone();
        thread::spawn(move || {
            for _ in changes {
                let new = Look::read(&config);
                let old = std::mem::replace(
                    &mut *look.lock().unwrap_or_else(PoisonError::into_inner),
                    new,
                );
                if old == new {
                    continue;
                }
                let Ok(emitter) = SignalEmitter::new(connection.inner(), OBJECT_PATH) else {
                    continue;
                };
                let before: HashMap<_, _> = namespaces(old, &cursor)
                    .into_iter()
                    .flat_map(|(namespace, values)| {
                        values
                            .into_iter()
                            .map(move |(key, value)| ((namespace, key), value))
                    })
                    .collect();
                for (namespace, values) in namespaces(new, &cursor) {
                    for (key, value) in values {
                        if before.get(&(namespace, key)) != Some(&value) {
                            let _ = zbus::block_on(Settings::setting_changed(
                                &emitter,
                                namespace,
                                key,
                                Value::from(value),
                            ));
                        }
                    }
                }
            }
        });
        Ok(())
    }

    fn current(&self) -> Namespaces {
        let look = *self.look.lock().unwrap_or_else(PoisonError::into_inner);
        namespaces(look, &self.cursor)
    }
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.portal.Error")]
pub enum PortalError {
    #[zbus(error)]
    ZBus(zbus::Error),
    NotFound(String),
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Settings")]
impl Settings {
    fn read_all(&self, filters: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        self.current()
            .into_iter()
            .filter(|(namespace, _)| matches(namespace, &filters))
            .map(|(namespace, values)| {
                let values = values
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value))
                    .collect();
                (namespace.to_owned(), values)
            })
            .collect()
    }

    fn read(&self, namespace: String, key: String) -> Result<OwnedValue, PortalError> {
        self.current()
            .into_iter()
            .filter(|(name, _)| *name == namespace)
            .flat_map(|(_, values)| values)
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value)
            .ok_or_else(|| PortalError::NotFound(format!("{namespace} {key} is not set")))
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        VERSION
    }

    #[zbus(signal)]
    async fn setting_changed(
        emitter: &SignalEmitter<'_>,
        namespace: &str,
        key: &str,
        value: Value<'_>,
    ) -> zbus::Result<()>;
}
