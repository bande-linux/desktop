use std::collections::{BTreeMap, HashMap};

use unbound_config::Output;
use zbus::blocking::Connection;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedValue;

use unbound_settings_panes::{SettingId, Value, WITH_DISPLAY};

#[zbus::proxy(
    interface = "org.unbound.Config",
    default_service = "org.unbound.Config",
    default_path = "/org/unbound/Config"
)]
pub trait Config {
    #[zbus(property)]
    fn appearance(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_appearance(&self, value: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn dock_edge(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_dock_edge(&self, value: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn idle_dim_after(&self) -> zbus::Result<u64>;
    #[zbus(property)]
    fn set_idle_dim_after(&self, seconds: u64) -> zbus::Result<()>;
    #[zbus(property)]
    fn idle_lock_after(&self) -> zbus::Result<u64>;
    #[zbus(property)]
    fn set_idle_lock_after(&self, seconds: u64) -> zbus::Result<()>;
    #[zbus(property)]
    fn idle_suspend_after(&self) -> zbus::Result<u64>;
    #[zbus(property)]
    fn set_idle_suspend_after(&self, seconds: u64) -> zbus::Result<()>;
    #[zbus(property)]
    fn idle_suspend_with_display(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_idle_suspend_with_display(&self, with_display: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn key_repeat_rate(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn set_key_repeat_rate(&self, rate: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn key_repeat_delay(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn set_key_repeat_delay(&self, milliseconds: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn outputs(&self) -> zbus::Result<HashMap<String, HashMap<String, OwnedValue>>>;
    fn set_output(
        &self,
        connector: &str,
        settings: HashMap<&str, zbus::zvariant::Value<'_>>,
    ) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.unbound.NotificationCenter",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
pub trait NotificationCenter {
    #[zbus(property)]
    fn do_not_disturb(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_do_not_disturb(&self, enabled: bool) -> zbus::Result<()>;
}

fn output(settings: &HashMap<String, OwnedValue>) -> Output {
    let number = |key: &str| {
        settings
            .get(key)
            .and_then(|value| f64::try_from(value).ok())
    };
    Output {
        scale: number("scale"),
        refresh_rate: number("refresh-rate"),
        variable_refresh_rate: settings
            .get("variable-refresh-rate")
            .and_then(|value| bool::try_from(value).ok()),
    }
}

pub const CONFIG_SETTINGS: [SettingId; 7] = [
    SettingId::Appearance,
    SettingId::DockEdge,
    SettingId::DimAfter,
    SettingId::LockAfter,
    SettingId::SleepAfter,
    SettingId::KeyRepeatRate,
    SettingId::KeyRepeatDelay,
];

pub struct SettingsClient {
    config: ConfigProxyBlocking<'static>,
    notifications: NotificationCenterProxyBlocking<'static>,
}

impl SettingsClient {
    pub fn connect(connection: &Connection) -> zbus::Result<Self> {
        Ok(Self {
            config: ConfigProxyBlocking::builder(connection)
                .cache_properties(CacheProperties::No)
                .build()?,
            notifications: NotificationCenterProxyBlocking::builder(connection)
                .cache_properties(CacheProperties::No)
                .build()?,
        })
    }

    pub fn outputs(&self) -> zbus::Result<BTreeMap<String, Output>> {
        Ok(self
            .config
            .outputs()?
            .iter()
            .map(|(connector, settings)| (connector.clone(), output(settings)))
            .collect())
    }

    pub fn set_output(&self, connector: &str, output: &Output) -> zbus::Result<()> {
        let mut settings = HashMap::new();
        if let Some(scale) = output.scale {
            settings.insert("scale", scale.into());
        }
        if let Some(rate) = output.refresh_rate {
            settings.insert("refresh-rate", rate.into());
        }
        if let Some(enabled) = output.variable_refresh_rate {
            settings.insert("variable-refresh-rate", enabled.into());
        }
        self.config.set_output(connector, settings)
    }

    pub fn get(&self, id: SettingId) -> zbus::Result<Value> {
        Ok(match id {
            SettingId::DoNotDisturb => Value::Flag(self.notifications.do_not_disturb()?),
            SettingId::Appearance => Value::Keyword(self.config.appearance()?),
            SettingId::DockEdge => Value::Keyword(self.config.dock_edge()?),
            SettingId::DimAfter => Value::Seconds(self.config.idle_dim_after()?),
            SettingId::LockAfter => Value::Seconds(self.config.idle_lock_after()?),
            SettingId::SleepAfter if self.config.idle_suspend_with_display()? => {
                Value::Keyword(WITH_DISPLAY.to_owned())
            }
            SettingId::SleepAfter => Value::Seconds(self.config.idle_suspend_after()?),
            SettingId::KeyRepeatRate => Value::Number(self.config.key_repeat_rate()?),
            SettingId::KeyRepeatDelay => Value::Number(self.config.key_repeat_delay()?),
            _ => return Err(zbus::Error::Unsupported),
        })
    }

    pub fn read(&self, ids: &[SettingId]) -> Vec<(SettingId, Value)> {
        ids.iter()
            .copied()
            .filter_map(|id| match self.get(id) {
                Ok(value) => Some((id, value)),
                Err(error) => {
                    tracing::warn!("cannot read {id:?}: {error}");
                    None
                }
            })
            .collect()
    }

    pub fn set(&self, id: SettingId, value: &Value) -> zbus::Result<()> {
        match (id, value) {
            (SettingId::Appearance, Value::Keyword(keyword)) => self.config.set_appearance(keyword),
            (SettingId::DockEdge, Value::Keyword(keyword)) => self.config.set_dock_edge(keyword),
            (SettingId::DimAfter, Value::Seconds(seconds)) => {
                self.config.set_idle_dim_after(*seconds)
            }
            (SettingId::LockAfter, Value::Seconds(seconds)) => {
                self.config.set_idle_lock_after(*seconds)
            }
            (SettingId::SleepAfter, Value::Keyword(keyword)) if keyword == WITH_DISPLAY => {
                self.config.set_idle_suspend_with_display(true)
            }
            (SettingId::SleepAfter, Value::Seconds(seconds)) => {
                self.config.set_idle_suspend_after(*seconds)
            }
            (SettingId::KeyRepeatRate, Value::Number(rate)) => {
                self.config.set_key_repeat_rate(*rate)
            }
            (SettingId::KeyRepeatDelay, Value::Number(milliseconds)) => {
                self.config.set_key_repeat_delay(*milliseconds)
            }
            (SettingId::DoNotDisturb, Value::Flag(enabled)) => {
                self.notifications.set_do_not_disturb(*enabled)
            }
            _ => Err(zbus::Error::Unsupported),
        }
    }
}
