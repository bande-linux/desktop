use std::collections::{BTreeMap, HashMap};

use unbound_config::Output;
use zbus::blocking::Connection;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedValue;

use unbound_settings_panes::{POINTER_SPEEDS, SettingId, Value, WITH_DISPLAY};

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
    fn input_sources(&self) -> zbus::Result<Vec<String>>;
    #[zbus(property)]
    fn set_input_sources(&self, codes: Vec<String>) -> zbus::Result<()>;
    #[zbus(property)]
    fn caps_lock(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_caps_lock(&self, value: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn trackpad_tap_to_click(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_trackpad_tap_to_click(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn trackpad_natural_scrolling(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_trackpad_natural_scrolling(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn trackpad_speed(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn set_trackpad_speed(&self, speed: f64) -> zbus::Result<()>;
    #[zbus(property)]
    fn trackpad_secondary_click(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_trackpad_secondary_click(&self, value: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn trackpad_disable_while_typing(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_trackpad_disable_while_typing(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn mouse_natural_scrolling(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_mouse_natural_scrolling(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn mouse_speed(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn set_mouse_speed(&self, speed: f64) -> zbus::Result<()>;
    #[zbus(property)]
    fn alert_sound(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_alert_sound(&self, value: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn alert_volume(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn set_alert_volume(&self, volume: f64) -> zbus::Result<()>;
    #[zbus(property)]
    fn sound_effects(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_sound_effects(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn volume_feedback(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_volume_feedback(&self, on: bool) -> zbus::Result<()>;
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

pub const CONFIG_SETTINGS: [SettingId; 25] = [
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
    SettingId::InputSources,
    SettingId::CapsLock,
    SettingId::TrackpadTapToClick,
    SettingId::TrackpadNaturalScrolling,
    SettingId::TrackpadSpeed,
    SettingId::TrackpadSecondaryClick,
    SettingId::TrackpadDisableWhileTyping,
    SettingId::MouseNaturalScrolling,
    SettingId::MouseSpeed,
    SettingId::AlertSound,
    SettingId::AlertVolume,
    SettingId::SoundEffects,
    SettingId::VolumeFeedback,
];

pub fn speed_stop(speed: f64) -> u32 {
    let stops = f64::from(POINTER_SPEEDS);
    ((speed.clamp(-1.0, 1.0) + 1.0) / 2.0 * stops).round() as u32
}

pub fn stop_speed(stop: u32) -> f64 {
    let stops = f64::from(POINTER_SPEEDS);
    f64::from(stop.min(POINTER_SPEEDS)) / stops * 2.0 - 1.0
}

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
            SettingId::InputSources => Value::Codes(self.config.input_sources()?),
            SettingId::CapsLock => Value::Keyword(self.config.caps_lock()?),
            SettingId::TrackpadTapToClick => Value::Flag(self.config.trackpad_tap_to_click()?),
            SettingId::TrackpadNaturalScrolling => {
                Value::Flag(self.config.trackpad_natural_scrolling()?)
            }
            SettingId::TrackpadSpeed => Value::Number(speed_stop(self.config.trackpad_speed()?)),
            SettingId::TrackpadSecondaryClick => {
                Value::Keyword(self.config.trackpad_secondary_click()?)
            }
            SettingId::TrackpadDisableWhileTyping => {
                Value::Flag(self.config.trackpad_disable_while_typing()?)
            }
            SettingId::MouseNaturalScrolling => Value::Flag(self.config.mouse_natural_scrolling()?),
            SettingId::MouseSpeed => Value::Number(speed_stop(self.config.mouse_speed()?)),
            SettingId::AlertSound => Value::Keyword(self.config.alert_sound()?),
            SettingId::AlertVolume => Value::Fraction(self.config.alert_volume()? as f32),
            SettingId::SoundEffects => Value::Flag(self.config.sound_effects()?),
            SettingId::VolumeFeedback => Value::Flag(self.config.volume_feedback()?),
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
            (SettingId::InputSources, Value::Codes(codes)) => {
                self.config.set_input_sources(codes.clone())
            }
            (SettingId::CapsLock, Value::Keyword(keyword)) => self.config.set_caps_lock(keyword),
            (SettingId::AlertSound, Value::Keyword(keyword)) => {
                self.config.set_alert_sound(keyword)?;
                unbound_sound::play(
                    self.config.inner().connection(),
                    &format!("alert-{keyword}"),
                );
                Ok(())
            }
            (SettingId::AlertVolume, Value::Fraction(volume)) => {
                self.config.set_alert_volume(f64::from(*volume))
            }
            (SettingId::SoundEffects, Value::Flag(on)) => self.config.set_sound_effects(*on),
            (SettingId::VolumeFeedback, Value::Flag(on)) => self.config.set_volume_feedback(*on),
            (SettingId::TrackpadTapToClick, Value::Flag(on)) => {
                self.config.set_trackpad_tap_to_click(*on)
            }
            (SettingId::TrackpadNaturalScrolling, Value::Flag(on)) => {
                self.config.set_trackpad_natural_scrolling(*on)
            }
            (SettingId::TrackpadSpeed, Value::Number(stop)) => {
                self.config.set_trackpad_speed(stop_speed(*stop))
            }
            (SettingId::TrackpadSecondaryClick, Value::Keyword(keyword)) => {
                self.config.set_trackpad_secondary_click(keyword)
            }
            (SettingId::TrackpadDisableWhileTyping, Value::Flag(on)) => {
                self.config.set_trackpad_disable_while_typing(*on)
            }
            (SettingId::MouseNaturalScrolling, Value::Flag(on)) => {
                self.config.set_mouse_natural_scrolling(*on)
            }
            (SettingId::MouseSpeed, Value::Number(stop)) => {
                self.config.set_mouse_speed(stop_speed(*stop))
            }
            (SettingId::DoNotDisturb, Value::Flag(enabled)) => {
                self.notifications.set_do_not_disturb(*enabled)
            }
            _ => Err(zbus::Error::Unsupported),
        }
    }
}
