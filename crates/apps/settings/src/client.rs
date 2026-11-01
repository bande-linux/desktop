use std::collections::{BTreeMap, HashMap};

use bande_config::{ArrangedDisplay, DisplaySettings, NightLightValue, Placement, Rotation};
use bande_datetime::DateFormats;
use bande_notify::client::NotificationCenter;
use zbus::blocking::Connection;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedValue;

use bande_settings_panes::{POINTER_SPEEDS, SettingId, Value, WITH_DISPLAY};

#[zbus::proxy(
    interface = "org.bande.Config",
    default_service = "org.bande.Config",
    default_path = "/org/bande/Config"
)]
pub trait Config {
    #[zbus(property)]
    fn appearance(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_appearance(&self, value: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn accent(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_accent(&self, value: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn tinted_frame(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_tinted_frame(&self, tinted: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn clock_format(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_clock_format(&self, value: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn low_power_on_battery(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_low_power_on_battery(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn lid_on_power(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_lid_on_power(&self, value: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn screen_reader(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_screen_reader(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn zoom(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn set_zoom(&self, zoom: f64) -> zbus::Result<()>;
    #[zbus(property)]
    fn increase_contrast(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_increase_contrast(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn reduce_transparency(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_reduce_transparency(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn reduce_motion(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_reduce_motion(&self, reduce: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn text_size(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn set_text_size(&self, percent: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn night_light(&self) -> zbus::Result<NightLightValue>;
    #[zbus(property)]
    fn set_night_light(&self, value: NightLightValue) -> zbus::Result<()>;
    #[zbus(property)]
    fn cursor_size(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn set_cursor_size(&self, size: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn flash_screen(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_flash_screen(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn sticky_keys(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_sticky_keys(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn slow_keys(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_slow_keys(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn bounce_keys(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_bounce_keys(&self, on: bool) -> zbus::Result<()>;
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
    fn bar_icon_size(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn set_bar_icon_size(&self, size: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn bar_auto_hide(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_bar_auto_hide(&self, auto_hide: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn bar_edge(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_bar_edge(&self, value: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn bar_menu_bar(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_bar_menu_bar(&self, value: &str) -> zbus::Result<()>;
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
    fn trackpad_workspace_swipe(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_trackpad_workspace_swipe(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn trackpad_overview_swipe(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_trackpad_overview_swipe(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn trackpad_desktop_spread(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_trackpad_desktop_spread(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn trackpad_launcher_pinch(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_trackpad_launcher_pinch(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn trackpad_acceleration(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_trackpad_acceleration(&self, on: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn trackpad_dragging(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_trackpad_dragging(&self, value: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn trackpad_scroll_method(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_trackpad_scroll_method(&self, value: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn mouse_primary_button(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_mouse_primary_button(&self, value: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn mouse_acceleration(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_mouse_acceleration(&self, on: bool) -> zbus::Result<()>;
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

pub const CONFIG_SETTINGS: [SettingId; 56] = [
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
    SettingId::TextSize,
    SettingId::NightLight,
    SettingId::NightSchedule,
    SettingId::NightFrom,
    SettingId::NightTo,
    SettingId::NightWarmth,
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
    SettingId::TrackpadWorkspaceSwipe,
    SettingId::TrackpadOverviewSwipe,
    SettingId::TrackpadDesktopSpread,
    SettingId::TrackpadLauncherPinch,
    SettingId::TrackpadAcceleration,
    SettingId::TrackpadDragging,
    SettingId::TrackpadScrollMethod,
    SettingId::MousePrimaryButton,
    SettingId::MouseNaturalScrolling,
    SettingId::MouseSpeed,
    SettingId::MouseAcceleration,
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
    config: ConfigProxyBlocking<'static>,
    notifications: NotificationCenter<'static>,
}

impl SettingsClient {
    pub fn connect(connection: &Connection) -> zbus::Result<Self> {
        Ok(Self {
            config: ConfigProxyBlocking::builder(connection)
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

    fn change_night_light(&self, change: impl FnOnce(&mut NightLightValue)) -> zbus::Result<()> {
        let mut night = self.config.night_light()?;
        change(&mut night);
        self.config.set_night_light(night)
    }

    pub fn get(&self, id: SettingId) -> zbus::Result<Value> {
        Ok(match id {
            SettingId::NightLight => Value::Flag(self.config.night_light()?.0),
            SettingId::NightSchedule => Value::Keyword(self.config.night_light()?.1),
            SettingId::NightFrom => Value::Text(self.config.night_light()?.2),
            SettingId::NightTo => Value::Text(self.config.night_light()?.3),
            SettingId::NightWarmth => Value::Number(self.config.night_light()?.4),
            SettingId::TextSize => Value::Number(self.config.text_size()?),
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
            SettingId::TrackpadWorkspaceSwipe => {
                Value::Flag(self.config.trackpad_workspace_swipe()?)
            }
            SettingId::TrackpadOverviewSwipe => Value::Flag(self.config.trackpad_overview_swipe()?),
            SettingId::TrackpadDesktopSpread => Value::Flag(self.config.trackpad_desktop_spread()?),
            SettingId::TrackpadLauncherPinch => Value::Flag(self.config.trackpad_launcher_pinch()?),
            SettingId::MouseNaturalScrolling => Value::Flag(self.config.mouse_natural_scrolling()?),
            SettingId::MouseSpeed => Value::Number(speed_stop(self.config.mouse_speed()?)),
            SettingId::TrackpadAcceleration => Value::Flag(self.config.trackpad_acceleration()?),
            SettingId::TrackpadDragging => Value::Keyword(self.config.trackpad_dragging()?),
            SettingId::TrackpadScrollMethod => {
                Value::Keyword(self.config.trackpad_scroll_method()?)
            }
            SettingId::MousePrimaryButton => Value::Keyword(self.config.mouse_primary_button()?),
            SettingId::MouseAcceleration => Value::Flag(self.config.mouse_acceleration()?),
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
            (SettingId::TextSize, Value::Number(percent)) => self.config.set_text_size(*percent),
            (SettingId::NightLight, Value::Flag(on)) => {
                self.change_night_light(|night| night.0 = *on)
            }
            (SettingId::NightSchedule, Value::Keyword(schedule)) => {
                self.change_night_light(|night| schedule.clone_into(&mut night.1))
            }
            (SettingId::NightFrom, Value::Text(from)) => {
                self.change_night_light(|night| from.clone_into(&mut night.2))
            }
            (SettingId::NightTo, Value::Text(to)) => {
                self.change_night_light(|night| to.clone_into(&mut night.3))
            }
            (SettingId::NightWarmth, Value::Number(warmth)) => {
                self.change_night_light(|night| night.4 = *warmth)
            }
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
                bande_sound::play(
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
            (SettingId::TrackpadWorkspaceSwipe, Value::Flag(on)) => {
                self.config.set_trackpad_workspace_swipe(*on)
            }
            (SettingId::TrackpadOverviewSwipe, Value::Flag(on)) => {
                self.config.set_trackpad_overview_swipe(*on)
            }
            (SettingId::TrackpadDesktopSpread, Value::Flag(on)) => {
                self.config.set_trackpad_desktop_spread(*on)
            }
            (SettingId::TrackpadLauncherPinch, Value::Flag(on)) => {
                self.config.set_trackpad_launcher_pinch(*on)
            }
            (SettingId::MouseNaturalScrolling, Value::Flag(on)) => {
                self.config.set_mouse_natural_scrolling(*on)
            }
            (SettingId::MouseSpeed, Value::Number(stop)) => {
                self.config.set_mouse_speed(stop_speed(*stop))
            }
            (SettingId::TrackpadAcceleration, Value::Flag(on)) => {
                self.config.set_trackpad_acceleration(*on)
            }
            (SettingId::TrackpadDragging, Value::Keyword(keyword)) => {
                self.config.set_trackpad_dragging(keyword)
            }
            (SettingId::TrackpadScrollMethod, Value::Keyword(keyword)) => {
                self.config.set_trackpad_scroll_method(keyword)
            }
            (SettingId::MousePrimaryButton, Value::Keyword(keyword)) => {
                self.config.set_mouse_primary_button(keyword)
            }
            (SettingId::MouseAcceleration, Value::Flag(on)) => {
                self.config.set_mouse_acceleration(*on)
            }
            (SettingId::DoNotDisturb, Value::Flag(enabled)) => {
                self.notifications.set_do_not_disturb(*enabled)
            }
            _ => Err(zbus::Error::Unsupported),
        }
    }
}
