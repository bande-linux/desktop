use zbus::blocking::Connection;

use crate::registry::{SettingId, Value, WITH_DISPLAY};

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
}

impl SettingsClient {
    pub fn connect(connection: &Connection) -> zbus::Result<Self> {
        Ok(Self {
            config: ConfigProxyBlocking::new(connection)?,
        })
    }

    pub fn proxy(&self) -> &ConfigProxyBlocking<'static> {
        &self.config
    }

    pub fn get(&self, id: SettingId) -> zbus::Result<Value> {
        Ok(match id {
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
        })
    }

    pub fn read_all(&self) -> Vec<(SettingId, Value)> {
        CONFIG_SETTINGS
            .into_iter()
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
            _ => Err(zbus::Error::Unsupported),
        }
    }
}
