use std::collections::{BTreeMap, HashMap};

use unbound_config::{ArrangedDisplay, DisplaySettings, Placement, Rotation};

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
    fn reduce_motion(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_reduce_motion(&self, reduce: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn wallpaper_light(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_wallpaper_light(&self, path: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn wallpaper_dark(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_wallpaper_dark(&self, path: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn wallpaper_fit(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_wallpaper_fit(&self, value: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn dock_icon_size(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn set_dock_icon_size(&self, size: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn dock_auto_hide(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_dock_auto_hide(&self, auto_hide: bool) -> zbus::Result<()>;
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
    fn displays(&self) -> zbus::Result<HashMap<String, HashMap<String, OwnedValue>>>;
    fn set_display(
        &self,
        identity: &str,
        settings: HashMap<&str, zbus::zvariant::Value<'_>>,
    ) -> zbus::Result<()>;
    #[zbus(property)]
    fn arrangements(&self) -> zbus::Result<Vec<Vec<ArrangedDisplay>>>;
    fn set_arrangement(&self, displays: Vec<ArrangedDisplay>) -> zbus::Result<()>;
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

fn display_settings(settings: &HashMap<String, OwnedValue>) -> DisplaySettings {
    let number = |key: &str| {
        settings
            .get(key)
            .and_then(|value| f64::try_from(value).ok())
    };
    DisplaySettings {
        resolution: settings
            .get("resolution")
            .and_then(|value| value.try_clone().ok())
            .and_then(|value| <(u32, u32)>::try_from(value).ok()),
        scale: number("scale"),
        refresh_rate: number("refresh-rate"),
        rotation: settings
            .get("rotation")
            .and_then(|value| u32::try_from(value).ok())
            .and_then(Rotation::from_degrees),
        variable_refresh_rate: settings
            .get("variable-refresh-rate")
            .and_then(|value| bool::try_from(value).ok()),
    }
}

pub const CONFIG_SETTINGS: [SettingId; 12] = [
    SettingId::Appearance,
    SettingId::ReduceMotion,
    SettingId::Wallpaper,
    SettingId::WallpaperFit,
    SettingId::DockIconSize,
    SettingId::DockAutoHide,
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

    pub fn displays(&self) -> zbus::Result<BTreeMap<String, DisplaySettings>> {
        Ok(self
            .config
            .displays()?
            .iter()
            .map(|(identity, settings)| (identity.clone(), display_settings(settings)))
            .collect())
    }

    pub fn set_display(&self, identity: &str, display: &DisplaySettings) -> zbus::Result<()> {
        let mut settings = HashMap::new();
        if let Some(resolution) = display.resolution {
            settings.insert("resolution", resolution.into());
        }
        if let Some(scale) = display.scale {
            settings.insert("scale", scale.into());
        }
        if let Some(rate) = display.refresh_rate {
            settings.insert("refresh-rate", rate.into());
        }
        if let Some(rotation) = display.rotation {
            settings.insert("rotation", rotation.degrees().into());
        }
        if let Some(enabled) = display.variable_refresh_rate {
            settings.insert("variable-refresh-rate", enabled.into());
        }
        self.config.set_display(identity, settings)
    }

    pub fn arrangements(&self) -> zbus::Result<Vec<Vec<Placement>>> {
        Ok(self
            .config
            .arrangements()?
            .into_iter()
            .map(|arranged| {
                arranged
                    .into_iter()
                    .map(|(display, x, y, main, mirror)| Placement {
                        display,
                        x,
                        y,
                        main,
                        mirror: (!mirror.is_empty()).then_some(mirror),
                    })
                    .collect()
            })
            .collect())
    }

    pub fn set_arrangement(&self, placements: &[Placement]) -> zbus::Result<()> {
        self.config.set_arrangement(
            placements
                .iter()
                .map(|placement| {
                    (
                        placement.display.clone(),
                        placement.x,
                        placement.y,
                        placement.main,
                        placement.mirror.clone().unwrap_or_default(),
                    )
                })
                .collect(),
        )
    }

    pub fn get(&self, id: SettingId) -> zbus::Result<Value> {
        Ok(match id {
            SettingId::DoNotDisturb => Value::Flag(self.notifications.do_not_disturb()?),
            SettingId::Appearance => Value::Keyword(self.config.appearance()?),
            SettingId::ReduceMotion => Value::Flag(self.config.reduce_motion()?),
            SettingId::Wallpaper => Value::Pictures {
                light: self.config.wallpaper_light()?,
                dark: self.config.wallpaper_dark()?,
            },
            SettingId::WallpaperFit => Value::Keyword(self.config.wallpaper_fit()?),
            SettingId::DockIconSize => Value::Number(self.config.dock_icon_size()?),
            SettingId::DockAutoHide => Value::Flag(self.config.dock_auto_hide()?),
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
            (SettingId::ReduceMotion, Value::Flag(reduce)) => {
                self.config.set_reduce_motion(*reduce)
            }
            (SettingId::Wallpaper, Value::Pictures { light, dark }) => {
                self.config.set_wallpaper_light(light)?;
                self.config.set_wallpaper_dark(dark)
            }
            (SettingId::WallpaperFit, Value::Keyword(keyword)) => {
                self.config.set_wallpaper_fit(keyword)
            }
            (SettingId::DockIconSize, Value::Number(size)) => self.config.set_dock_icon_size(*size),
            (SettingId::DockAutoHide, Value::Flag(auto_hide)) => {
                self.config.set_dock_auto_hide(*auto_hide)
            }

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
