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
pub const UNBOUND_APPEARANCE: &str = "org.unbound.appearance";
pub const GNOME_SOUND: &str = "org.gnome.desktop.sound";
pub const SOUND_THEME: &str = "Unbound";
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
        let accent = Palette::for_scheme(scheme).accent;
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

pub fn cursor_theme_from_environment() -> String {
    std::env::var("XCURSOR_THEME")
        .ok()
        .filter(|theme| !theme.is_empty())
        .unwrap_or_else(|| "default".to_owned())
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Look {
    scheme: ColorScheme,
    increase_contrast: bool,
    reduce_transparency: bool,
    reduce_motion: bool,
    text_size: u32,
    cursor_size: u32,
}

impl Look {
    fn read(config: &ConfigProxyBlocking<'_>) -> Self {
        Self {
            scheme: ColorScheme::from_appearance(&config.appearance().unwrap_or_default()),
            increase_contrast: config.increase_contrast().unwrap_or(false),
            reduce_transparency: config.reduce_transparency().unwrap_or(false),
            reduce_motion: config.reduce_motion().unwrap_or(false),
            text_size: config.text_size().unwrap_or(100),
            cursor_size: config.cursor_size().unwrap_or(24),
        }
    }
}

type Namespaces = Vec<(&'static str, Vec<(&'static str, OwnedValue)>)>;

fn namespaces(look: Look, cursor_theme: &str) -> Namespaces {
    let scheme = look.scheme;
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
                ("contrast", Value::from(u32::from(look.increase_contrast))),
                ("reduced-motion", Value::from(u32::from(look.reduce_motion))),
            ]),
        ),
        (
            UNBOUND_APPEARANCE,
            owned(vec![
                (
                    "reduce-transparency",
                    Value::from(u32::from(look.reduce_transparency)),
                ),
                ("text-size", Value::from(look.text_size)),
            ]),
        ),
        (
            GNOME_INTERFACE,
            owned(vec![
                ("color-scheme", Value::from(scheme.gnome_color_scheme())),
                ("cursor-theme", Value::from(cursor_theme)),
                (
                    "cursor-size",
                    Value::from(i32::try_from(look.cursor_size).unwrap_or(24)),
                ),
                (
                    "text-scaling-factor",
                    Value::from(f64::from(look.text_size) / 100.0),
                ),
            ]),
        ),
        (
            GNOME_SOUND,
            owned(vec![
                ("theme-name", Value::from(SOUND_THEME)),
                ("event-sounds", Value::from(true)),
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
    fn increase_contrast(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn reduce_transparency(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn reduce_motion(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn text_size(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn cursor_size(&self) -> zbus::Result<u32>;
}

pub struct Settings {
    look: Arc<Mutex<Look>>,
    cursor_theme: String,
}

impl Settings {
    pub fn serve(connection: &Connection, cursor_theme: String) -> zbus::Result<()> {
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
                cursor_theme: cursor_theme.clone(),
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
                let before: HashMap<_, _> = namespaces(old, &cursor_theme)
                    .into_iter()
                    .flat_map(|(namespace, values)| {
                        values
                            .into_iter()
                            .map(move |(key, value)| ((namespace, key), value))
                    })
                    .collect();
                for (namespace, values) in namespaces(new, &cursor_theme) {
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
        namespaces(look, &self.cursor_theme)
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
