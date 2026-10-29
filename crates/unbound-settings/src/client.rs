use zbus::blocking::Connection;

use crate::model::{Key, Value};

#[zbus::proxy(
    interface = "org.unbound.Config",
    default_service = "org.unbound.Config",
    default_path = "/org/unbound/Config"
)]
trait Config {
    #[zbus(property)]
    fn appearance(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_appearance(&self, value: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn panel_edge(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_panel_edge(&self, value: &str) -> zbus::Result<()>;
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
}

pub struct SettingsClient {
    config: ConfigProxyBlocking<'static>,
}

impl SettingsClient {
    pub fn connect(connection: &Connection) -> zbus::Result<Self> {
        Ok(Self {
            config: ConfigProxyBlocking::new(connection)?,
        })
    }

    pub fn get(&self, key: Key) -> zbus::Result<Value> {
        Ok(match key {
            Key::Appearance => Value::Keyword(self.config.appearance()?),
            Key::DockPosition => Value::Keyword(self.config.panel_edge()?),
            Key::DimAfter => Value::Seconds(self.config.idle_dim_after()?),
            Key::LockAfter => Value::Seconds(self.config.idle_lock_after()?),
            Key::SleepAfter => Value::Seconds(self.config.idle_suspend_after()?),
        })
    }

    pub fn set(&self, key: Key, value: &Value) -> zbus::Result<()> {
        match (key, value) {
            (Key::Appearance, Value::Keyword(keyword)) => self.config.set_appearance(keyword),
            (Key::DockPosition, Value::Keyword(keyword)) => self.config.set_panel_edge(keyword),
            (Key::DimAfter, Value::Seconds(seconds)) => self.config.set_idle_dim_after(*seconds),
            (Key::LockAfter, Value::Seconds(seconds)) => self.config.set_idle_lock_after(*seconds),
            (Key::SleepAfter, Value::Seconds(seconds)) => {
                self.config.set_idle_suspend_after(*seconds)
            }
            _ => Err(zbus::Error::Unsupported),
        }
    }
}
