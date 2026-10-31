use std::collections::{BTreeMap, HashMap};

use unbound_config::client::ConfigProxy;
use unbound_config::{DisplaySettings, Placement, Rotation};
use unbound_datetime::DateFormats;
use unbound_notify::client::NotificationCenter;
use zbus::blocking::Connection;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedValue;

use unbound_settings_panes::{POINTER_SPEEDS, SettingId, Value, WITH_DISPLAY};

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

pub const CONFIG_SETTINGS: [SettingId; 41] = [
    SettingId::Appearance,
    SettingId::Clock24Hour,
    SettingId::LowPowerOnBattery,
    SettingId::LidOnPower,
    SettingId::ScreenReader,
    SettingId::Zoom,
    SettingId::IncreaseContrast,
    SettingId::ReduceTransparency,
    SettingId::ReduceMotion,
    SettingId::CursorSize,
    SettingId::FlashScreen,
    SettingId::StickyKeys,
    SettingId::SlowKeys,
    SettingId::BounceKeys,
    SettingId::Accent,
    SettingId::TintedFrame,
    SettingId::Wallpaper,
    SettingId::WallpaperFit,
    SettingId::BarIconSize,
    SettingId::BarAutoHide,
    SettingId::BarNames,
    SettingId::BarEdge,
    SettingId::BarMenuBar,
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

const ZOOM_WHEN_TURNED_ON: f64 = 2.0;

pub fn speed_stop(speed: f64) -> u32 {
    let stops = f64::from(POINTER_SPEEDS);
    ((speed.clamp(-1.0, 1.0) + 1.0) / 2.0 * stops).round() as u32
}

pub fn stop_speed(stop: u32) -> f64 {
    let stops = f64::from(POINTER_SPEEDS);
    f64::from(stop.min(POINTER_SPEEDS)) / stops * 2.0 - 1.0
}

pub struct SettingsClient {
    config: ConfigProxy<'static>,
    notifications: NotificationCenter<'static>,
}

impl SettingsClient {
    pub fn connect(connection: &Connection) -> zbus::Result<Self> {
        Ok(Self {
            config: ConfigProxy::builder(connection)
                .cache_properties(CacheProperties::No)
                .build()?,
            notifications: NotificationCenter::builder(connection)
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
            SettingId::ScreenReader => Value::Flag(self.config.screen_reader()?),
            SettingId::Zoom => Value::Flag(self.config.zoom()? > 1.0),
            SettingId::IncreaseContrast => Value::Flag(self.config.increase_contrast()?),
            SettingId::ReduceTransparency => Value::Flag(self.config.reduce_transparency()?),
            SettingId::ReduceMotion => Value::Flag(self.config.reduce_motion()?),
            SettingId::CursorSize => Value::Number(self.config.cursor_size()?),
            SettingId::FlashScreen => Value::Flag(self.config.flash_screen()?),
            SettingId::StickyKeys => Value::Flag(self.config.sticky_keys()?),
            SettingId::SlowKeys => Value::Flag(self.config.slow_keys()?),
            SettingId::BounceKeys => Value::Flag(self.config.bounce_keys()?),
            SettingId::Accent => Value::Keyword(self.config.accent()?),
            SettingId::TintedFrame => Value::Flag(self.config.tinted_frame()?),
            SettingId::LowPowerOnBattery => Value::Flag(self.config.low_power_on_battery()?),
            SettingId::LidOnPower => Value::Keyword(self.config.lid_on_power()?),
            SettingId::Clock24Hour => Value::Flag(match self.config.clock_format()?.as_str() {
                "24-hour" => true,
                "12-hour" => false,
                _ => DateFormats::from_env().uses_24_hour_clock(),
            }),
            SettingId::Wallpaper => Value::Pictures {
                light: self.config.wallpaper_light()?,
                dark: self.config.wallpaper_dark()?,
            },
            SettingId::WallpaperFit => Value::Keyword(self.config.wallpaper_fit()?),
            SettingId::BarIconSize => Value::Number(self.config.bar_icon_size()?),
            SettingId::BarAutoHide => Value::Flag(self.config.bar_auto_hide()?),
            SettingId::BarNames => Value::Flag(self.config.bar_names()?),
            SettingId::BarEdge => Value::Keyword(self.config.bar_edge()?),
            SettingId::BarMenuBar => Value::Keyword(self.config.bar_menu_bar()?),
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
            (SettingId::Accent, Value::Keyword(keyword)) => self.config.set_accent(keyword),
            (SettingId::TintedFrame, Value::Flag(tinted)) => self.config.set_tinted_frame(*tinted),
            (SettingId::BarEdge, Value::Keyword(keyword)) => self.config.set_bar_edge(keyword),
            (SettingId::BarMenuBar, Value::Keyword(keyword)) => {
                self.config.set_bar_menu_bar(keyword)
            }
            (SettingId::Clock24Hour, Value::Flag(twenty_four_hour)) => {
                self.config.set_clock_format(if *twenty_four_hour {
                    "24-hour"
                } else {
                    "12-hour"
                })
            }
            (SettingId::LowPowerOnBattery, Value::Flag(on)) => {
                self.config.set_low_power_on_battery(*on)
            }
            (SettingId::LidOnPower, Value::Keyword(keyword)) => {
                self.config.set_lid_on_power(keyword)
            }
            (SettingId::ScreenReader, Value::Flag(on)) => self.config.set_screen_reader(*on),
            (SettingId::Zoom, Value::Flag(on)) => {
                self.config
                    .set_zoom(if *on { ZOOM_WHEN_TURNED_ON } else { 1.0 })
            }
            (SettingId::IncreaseContrast, Value::Flag(on)) => {
                self.config.set_increase_contrast(*on)
            }
            (SettingId::ReduceTransparency, Value::Flag(on)) => {
                self.config.set_reduce_transparency(*on)
            }
            (SettingId::ReduceMotion, Value::Flag(reduce)) => {
                self.config.set_reduce_motion(*reduce)
            }
            (SettingId::CursorSize, Value::Number(size)) => self.config.set_cursor_size(*size),
            (SettingId::FlashScreen, Value::Flag(on)) => self.config.set_flash_screen(*on),
            (SettingId::StickyKeys, Value::Flag(on)) => self.config.set_sticky_keys(*on),
            (SettingId::SlowKeys, Value::Flag(on)) => self.config.set_slow_keys(*on),
            (SettingId::BounceKeys, Value::Flag(on)) => self.config.set_bounce_keys(*on),
            (SettingId::Wallpaper, Value::Pictures { light, dark }) => {
                self.config.set_wallpaper_light(light)?;
                self.config.set_wallpaper_dark(dark)
            }
            (SettingId::WallpaperFit, Value::Keyword(keyword)) => {
                self.config.set_wallpaper_fit(keyword)
            }
            (SettingId::BarIconSize, Value::Number(size)) => self.config.set_bar_icon_size(*size),
            (SettingId::BarAutoHide, Value::Flag(auto_hide)) => {
                self.config.set_bar_auto_hide(*auto_hide)
            }
            (SettingId::BarNames, Value::Flag(names)) => self.config.set_bar_names(*names),

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
            (SettingId::InputSources, Value::Codes(codes)) => self.config.set_input_sources(codes),
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
