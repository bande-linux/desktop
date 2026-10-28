use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use unbound_toolkit::{Palette, Scheme};
use zbus::blocking::Connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

use crate::OBJECT_PATH;

pub const APPEARANCE: &str = "org.freedesktop.appearance";
pub const VERSION: u32 = 2;

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

    fn values(self) -> Vec<(&'static str, OwnedValue)> {
        let owned = |value: Value<'_>| OwnedValue::try_from(value).ok();
        [
            ("color-scheme", owned(Value::from(self as u32))),
            ("accent-color", owned(Value::from(self.accent_color()))),
            ("contrast", owned(Value::from(0u32))),
        ]
        .into_iter()
        .filter_map(|(key, value)| Some((key, value?)))
        .collect()
    }
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
}

pub struct Settings {
    scheme: Arc<Mutex<ColorScheme>>,
}

impl Settings {
    pub fn serve(connection: &Connection) -> zbus::Result<()> {
        let config = ConfigProxyBlocking::new(connection)?;
        let changes = config.receive_appearance_changed();
        let scheme = Arc::new(Mutex::new(ColorScheme::from_appearance(
            &config.appearance().unwrap_or_default(),
        )));
        connection.object_server().at(
            OBJECT_PATH,
            Settings {
                scheme: Arc::clone(&scheme),
            },
        )?;
        let connection = connection.clone();
        thread::spawn(move || {
            for change in changes {
                let Ok(appearance) = change.get() else {
                    continue;
                };
                let new = ColorScheme::from_appearance(&appearance);
                let old = std::mem::replace(
                    &mut *scheme.lock().unwrap_or_else(PoisonError::into_inner),
                    new,
                );
                let Ok(emitter) = SignalEmitter::new(connection.inner(), OBJECT_PATH) else {
                    continue;
                };
                let before: HashMap<_, _> = old.values().into_iter().collect();
                for (key, value) in new.values() {
                    if before.get(key) != Some(&value) {
                        let _ = zbus::block_on(Settings::setting_changed(
                            &emitter,
                            APPEARANCE,
                            key,
                            Value::from(value),
                        ));
                    }
                }
            }
        });
        Ok(())
    }

    fn current(&self) -> ColorScheme {
        *self.scheme.lock().unwrap_or_else(PoisonError::into_inner)
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
    fn read_all(&self, namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        if !matches(APPEARANCE, &namespaces) {
            return HashMap::new();
        }
        let values = self
            .current()
            .values()
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect();
        HashMap::from([(APPEARANCE.to_owned(), values)])
    }

    fn read(&self, namespace: String, key: String) -> Result<OwnedValue, PortalError> {
        let not_found = || PortalError::NotFound(format!("{namespace} {key} is not set"));
        if namespace != APPEARANCE {
            return Err(not_found());
        }
        self.current()
            .values()
            .into_iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value)
            .ok_or_else(not_found)
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
