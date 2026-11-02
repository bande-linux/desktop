use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use bande_watch::{Changes, FolderWatch};
use zbus::blocking::Connection;
use zbus::fdo::Error::{Failed, InvalidArgs};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

use crate::{
    ACCENTS, Accessibility, AlertSound, Appearance, BarEdge, BarLayout, CURSOR_SIZE_RANGE,
    CapsLock, ClockFormat, ClockTime, Config, ConfigStore, DisplaySettings, Dragging,
    FocusSchedule, InputSource, LidOnPower, Loaded, NIGHT_LIGHT_WARMTH_RANGE, NightLight,
    NightSchedule, Placement, PrimaryButton, ReloadError, Rotation, ScrollMethod, SearchCategory,
    SecondaryClick, Setting, Suspend, TEXT_SIZE_RANGE, WallpaperFit, WallpaperVariant, ZOOM_RANGE,
    accent_keyword, bar_placement, is_wallpaper_path, read_system_input_sources,
};

pub type ArrangedDisplay = (String, i32, i32, bool, String);
pub type NightLightValue = (bool, String, String, String, u32);
pub type FocusScheduleValue = (bool, String, String);

fn clock_time(text: &str) -> zbus::fdo::Result<ClockTime> {
    ClockTime::parse(text)
        .ok_or_else(|| InvalidArgs(format!("\"{text}\" is not a time of day such as \"22:00\"")))
}

pub const BUS_NAME: &str = "org.bande.Config";
pub const OBJECT_PATH: &str = "/org/bande/Config";

const STAGING_NAME: &str = ".config.kdl.new";

#[derive(Debug)]
pub enum ServiceError {
    Bus(zbus::Error),
    Watch(io::Error),
    Path(PathBuf),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bus(error) => write!(f, "D-Bus: {error}"),
            Self::Watch(error) => write!(f, "cannot watch the configuration: {error}"),
            Self::Path(path) => write!(f, "{} has no parent directory", path.display()),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<zbus::Error> for ServiceError {
    fn from(error: zbus::Error) -> Self {
        Self::Bus(error)
    }
}

impl From<io::Error> for ServiceError {
    fn from(error: io::Error) -> Self {
        Self::Watch(error)
    }
}

fn replace(path: &Path, contents: &str) -> io::Result<()> {
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let staging = target.with_file_name(STAGING_NAME);
    let mut file = File::create(&staging)?;
    if let Ok(existing) = std::fs::metadata(&target) {
        file.set_permissions(existing.permissions())?;
    }
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&staging, &target)
}

pub struct ConfigService {
    _watch: FolderWatch,
    _connection: Connection,
}

type Published = Arc<Mutex<Arc<Config>>>;

fn lock(published: &Published) -> MutexGuard<'_, Arc<Config>> {
    published.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ConfigService {
    pub fn start(
        connection: &Connection,
        path: &Path,
        store: Arc<ConfigStore>,
        on_reload: impl Fn(Result<Loaded, ReloadError>) + Send + 'static,
    ) -> Result<Self, ServiceError> {
        let directory = path
            .parent()
            .ok_or_else(|| ServiceError::Path(path.to_path_buf()))?;
        let file_name: OsString = path
            .file_name()
            .ok_or_else(|| ServiceError::Path(path.to_path_buf()))?
            .to_owned();
        std::fs::create_dir_all(directory)?;

        if path.exists() {
            on_reload(store.reload(path));
        }
        let published = Arc::new(Mutex::new(store.current()));

        connection.object_server().at(
            OBJECT_PATH,
            ConfigInterface {
                store: Arc::clone(&store),
                path: path.to_path_buf(),
                published: Arc::clone(&published),
            },
        )?;
        connection.request_name(BUS_NAME)?;
        let interface = connection
            .object_server()
            .interface::<_, ConfigInterface>(OBJECT_PATH)?;

        let watched = path.to_path_buf();
        let watch = FolderWatch::spawn(Changes::Saved, move |paths| {
            if !paths
                .iter()
                .any(|changed| changed.file_name() == Some(file_name.as_os_str()))
            {
                return;
            }
            let mut published = lock(&published);
            let reloaded = store.reload(&watched);
            if let Ok(loaded) = &reloaded {
                publish_changes(&interface, &published, &loaded.config);
                *published = Arc::clone(&loaded.config);
            }
            on_reload(reloaded);
        })?;
        watch.watch(directory)?;
        Ok(Self {
            _watch: watch,
            _connection: connection.clone(),
        })
    }
}

fn publish_changes(
    interface: &zbus::blocking::object_server::InterfaceRef<ConfigInterface>,
    before: &Config,
    after: &Config,
) {
    let emitter = interface.signal_emitter();
    let guard = interface.get();
    zbus::block_on(async {
        if before.appearance != after.appearance {
            let _ = guard.appearance_changed(emitter).await;
        }
        if before.accent != after.accent {
            let _ = guard.accent_changed(emitter).await;
        }
        if before.tinted_frame != after.tinted_frame {
            let _ = guard.tinted_frame_changed(emitter).await;
        }
        if before.reopen_apps != after.reopen_apps {
            let _ = guard.reopen_apps_changed(emitter).await;
        }
        if before.reopen_apps_after_crash != after.reopen_apps_after_crash {
            let _ = guard.reopen_apps_after_crash_changed(emitter).await;
        }
        if before.clock != after.clock {
            let _ = guard.clock_format_changed(emitter).await;
        }
        if before.power.show_battery_percentage != after.power.show_battery_percentage {
            let _ = guard.show_battery_percentage_changed(emitter).await;
        }
        if before.power.low_power_on_battery != after.power.low_power_on_battery {
            let _ = guard.low_power_on_battery_changed(emitter).await;
        }
        if before.power.lid_on_power != after.power.lid_on_power {
            let _ = guard.lid_on_power_changed(emitter).await;
        }
        let (was, is) = (&before.accessibility, &after.accessibility);
        if was.screen_reader != is.screen_reader {
            let _ = guard.screen_reader_changed(emitter).await;
        }
        if was.zoom != is.zoom {
            let _ = guard.zoom_changed(emitter).await;
        }
        if was.increase_contrast != is.increase_contrast {
            let _ = guard.increase_contrast_changed(emitter).await;
        }
        if was.reduce_transparency != is.reduce_transparency {
            let _ = guard.reduce_transparency_changed(emitter).await;
        }
        if was.reduce_motion != is.reduce_motion {
            let _ = guard.reduce_motion_changed(emitter).await;
        }
        if was.text_size != is.text_size {
            let _ = guard.text_size_changed(emitter).await;
        }
        if was.cursor_size != is.cursor_size {
            let _ = guard.cursor_size_changed(emitter).await;
        }
        if was.flash_screen != is.flash_screen {
            let _ = guard.flash_screen_changed(emitter).await;
        }
        if was.sticky_keys != is.sticky_keys {
            let _ = guard.sticky_keys_changed(emitter).await;
        }
        if was.slow_keys != is.slow_keys {
            let _ = guard.slow_keys_changed(emitter).await;
        }
        if was.bounce_keys != is.bounce_keys {
            let _ = guard.bounce_keys_changed(emitter).await;
        }
        if before.bar.edge != after.bar.edge {
            let _ = guard.bar_edge_changed(emitter).await;
        }
        if before.bar.apps != after.bar.apps {
            let _ = guard.bar_apps_changed(emitter).await;
        }
        if before.bar.auto_hide != after.bar.auto_hide {
            let _ = guard.bar_auto_hide_changed(emitter).await;
        }
        if before.bar.icon_size != after.bar.icon_size {
            let _ = guard.bar_icon_size_changed(emitter).await;
        }
        if before.bar.layout != after.bar.layout {
            let _ = guard.bar_layout_changed(emitter).await;
        }
        if before.idle.dim_after != after.idle.dim_after {
            let _ = guard.idle_dim_after_changed(emitter).await;
        }
        if before.idle.lock_after != after.idle.lock_after {
            let _ = guard.idle_lock_after_changed(emitter).await;
        }
        if before.idle.suspend_after() != after.idle.suspend_after() {
            let _ = guard.idle_suspend_after_changed(emitter).await;
        }
        if (before.idle.suspend == Suspend::WithDisplayOff)
            != (after.idle.suspend == Suspend::WithDisplayOff)
        {
            let _ = guard.idle_suspend_with_display_changed(emitter).await;
        }
        if before.keyboard.repeat_rate != after.keyboard.repeat_rate {
            let _ = guard.key_repeat_rate_changed(emitter).await;
        }
        if before.keyboard.repeat_delay != after.keyboard.repeat_delay {
            let _ = guard.key_repeat_delay_changed(emitter).await;
        }
        if before.keyboard.input_sources != after.keyboard.input_sources {
            let _ = guard.input_sources_changed(emitter).await;
        }
        if before.keyboard.caps_lock != after.keyboard.caps_lock {
            let _ = guard.caps_lock_changed(emitter).await;
        }
        if before.trackpad.tap_to_click != after.trackpad.tap_to_click {
            let _ = guard.trackpad_tap_to_click_changed(emitter).await;
        }
        if before.trackpad.natural_scrolling != after.trackpad.natural_scrolling {
            let _ = guard.trackpad_natural_scrolling_changed(emitter).await;
        }
        if before.trackpad.speed != after.trackpad.speed {
            let _ = guard.trackpad_speed_changed(emitter).await;
        }
        if before.trackpad.secondary_click != after.trackpad.secondary_click {
            let _ = guard.trackpad_secondary_click_changed(emitter).await;
        }
        if before.trackpad.disable_while_typing != after.trackpad.disable_while_typing {
            let _ = guard.trackpad_disable_while_typing_changed(emitter).await;
        }
        if before.trackpad.workspace_swipe != after.trackpad.workspace_swipe {
            let _ = guard.trackpad_workspace_swipe_changed(emitter).await;
        }
        if before.trackpad.overview_swipe != after.trackpad.overview_swipe {
            let _ = guard.trackpad_overview_swipe_changed(emitter).await;
        }
        if before.trackpad.desktop_spread != after.trackpad.desktop_spread {
            let _ = guard.trackpad_desktop_spread_changed(emitter).await;
        }
        if before.trackpad.launcher_pinch != after.trackpad.launcher_pinch {
            let _ = guard.trackpad_launcher_pinch_changed(emitter).await;
        }
        if before.mouse.natural_scrolling != after.mouse.natural_scrolling {
            let _ = guard.mouse_natural_scrolling_changed(emitter).await;
        }
        if before.mouse.speed != after.mouse.speed {
            let _ = guard.mouse_speed_changed(emitter).await;
        }
        if before.trackpad.acceleration != after.trackpad.acceleration {
            let _ = guard.trackpad_acceleration_changed(emitter).await;
        }
        if before.trackpad.dragging != after.trackpad.dragging {
            let _ = guard.trackpad_dragging_changed(emitter).await;
        }
        if before.trackpad.scroll_method != after.trackpad.scroll_method {
            let _ = guard.trackpad_scroll_method_changed(emitter).await;
        }
        if before.mouse.primary_button != after.mouse.primary_button {
            let _ = guard.mouse_primary_button_changed(emitter).await;
        }
        if before.mouse.acceleration != after.mouse.acceleration {
            let _ = guard.mouse_acceleration_changed(emitter).await;
        }
        if before.sound.effects != after.sound.effects {
            let _ = guard.sound_effects_changed(emitter).await;
        }
        if before.sound.volume_feedback != after.sound.volume_feedback {
            let _ = guard.volume_feedback_changed(emitter).await;
        }
        if before.sound.alert != after.sound.alert {
            let _ = guard.alert_sound_changed(emitter).await;
        }
        if before.sound.alert_volume != after.sound.alert_volume {
            let _ = guard.alert_volume_changed(emitter).await;
        }
        if before.wallpaper.light != after.wallpaper.light {
            let _ = guard.wallpaper_light_changed(emitter).await;
        }
        if before.wallpaper.dark != after.wallpaper.dark {
            let _ = guard.wallpaper_dark_changed(emitter).await;
        }
        if before.wallpaper.fit != after.wallpaper.fit {
            let _ = guard.wallpaper_fit_changed(emitter).await;
        }
        if before.focus_schedule != after.focus_schedule {
            let _ = guard.focus_schedule_changed(emitter).await;
        }
        if before.night_light != after.night_light {
            let _ = guard.night_light_changed(emitter).await;
        }
        if before.search.hidden != after.search.hidden {
            let _ = guard.search_shows_changed(emitter).await;
        }
        if before.search.exclude != after.search.exclude {
            let _ = guard.search_excluded_changed(emitter).await;
        }
        if before.displays != after.displays {
            let _ = guard.displays_changed(emitter).await;
        }
        if before.arrangements != after.arrangements {
            let _ = guard.arrangements_changed(emitter).await;
        }
    });
}

fn keyword<T: Copy>(
    choices: &[T],
    name: impl Fn(T) -> &'static str,
    value: &str,
) -> zbus::fdo::Result<T> {
    choices
        .iter()
        .copied()
        .find(|choice| name(*choice) == value)
        .ok_or_else(|| {
            let names: Vec<_> = choices
                .iter()
                .map(|choice| format!("\"{}\"", name(*choice)))
                .collect();
            InvalidArgs(format!("expected one of {}", names.join(", ")))
        })
}

fn display_settings(settings: &HashMap<String, OwnedValue>) -> zbus::fdo::Result<DisplaySettings> {
    let mut display = DisplaySettings::default();
    for (key, value) in settings {
        let mismatch = || InvalidArgs(format!("`{key}` has the wrong type"));
        match key.as_str() {
            "resolution" => {
                display.resolution = Some(
                    <(u32, u32)>::try_from(value.try_clone().map_err(|_| mismatch())?)
                        .map_err(|_| mismatch())?,
                )
            }
            "scale" => display.scale = Some(f64::try_from(value).map_err(|_| mismatch())?),
            "refresh-rate" => {
                display.refresh_rate = Some(f64::try_from(value).map_err(|_| mismatch())?)
            }
            "rotation" => {
                display.rotation = Some(
                    u32::try_from(value)
                        .ok()
                        .and_then(Rotation::from_degrees)
                        .ok_or_else(mismatch)?,
                )
            }
            "variable-refresh-rate" => {
                display.variable_refresh_rate = Some(bool::try_from(value).map_err(|_| mismatch())?)
            }
            _ => return Err(InvalidArgs(format!("unknown display setting `{key}`"))),
        }
    }
    Ok(display)
}

fn placement((display, x, y, main, mirror): ArrangedDisplay) -> Placement {
    Placement {
        display,
        x,
        y,
        main,
        mirror: (!mirror.is_empty()).then_some(mirror),
    }
}

fn arranged(placement: &Placement) -> ArrangedDisplay {
    (
        placement.display.clone(),
        placement.x,
        placement.y,
        placement.main,
        placement.mirror.clone().unwrap_or_default(),
    )
}

struct ConfigInterface {
    store: Arc<ConfigStore>,
    path: PathBuf,
    published: Published,
}

impl ConfigInterface {
    fn wallpaper(&self, variant: WallpaperVariant) -> String {
        self.store
            .current()
            .wallpaper
            .path(variant)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn set_wallpaper(&self, variant: WallpaperVariant, path: String) -> zbus::fdo::Result<()> {
        let path = (!path.is_empty()).then(|| PathBuf::from(path));
        self.write(&Setting::Wallpaper(variant, path))
    }

    fn write(&self, setting: &Setting) -> zbus::fdo::Result<()> {
        self.write_from(|_| setting.clone())
    }

    fn change_accessibility(
        &self,
        change: impl FnOnce(&mut Accessibility),
    ) -> zbus::fdo::Result<()> {
        self.write_from(|config| {
            let mut access = config.accessibility;
            change(&mut access);
            Setting::Accessibility(access)
        })
    }

    fn write_from(&self, setting: impl FnOnce(&Config) -> Setting) -> zbus::fdo::Result<()> {
        let mut published = lock(&self.published);
        let source = match std::fs::read_to_string(&self.path) {
            Ok(source) => source,
            Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(Failed(format!("cannot read the configuration: {error}")));
            }
        };
        let config = Config::parse(&source).map_err(|error| {
            Failed(format!(
                "the configuration is invalid at {error} and must be fixed first"
            ))
        })?;
        let edited = setting(&config)
            .apply(&source)
            .map_err(|error| InvalidArgs(error.message))?;
        replace(&self.path, &edited)
            .map_err(|error| Failed(format!("cannot write the configuration: {error}")))?;
        *published = self
            .store
            .apply(&edited)
            .map_err(|error| Failed(error.to_string()))?
            .config;
        Ok(())
    }
}

#[zbus::interface(name = "org.bande.Config")]
impl ConfigInterface {
    #[zbus(property)]
    fn appearance(&self) -> String {
        self.store.current().appearance.keyword().to_owned()
    }

    #[zbus(property)]
    fn set_appearance(&self, value: String) -> zbus::fdo::Result<()> {
        let appearance = keyword(
            &[Appearance::Dark, Appearance::Light, Appearance::Automatic],
            Appearance::keyword,
            &value,
        )?;
        self.write(&Setting::Appearance(appearance))
    }

    #[zbus(property)]
    fn accent(&self) -> String {
        accent_keyword(self.store.current().accent).to_owned()
    }

    #[zbus(property)]
    fn set_accent(&self, value: String) -> zbus::fdo::Result<()> {
        let accent = keyword(&ACCENTS.map(|(_, accent)| accent), accent_keyword, &value)?;
        self.write(&Setting::Accent(accent))
    }

    #[zbus(property)]
    fn tinted_frame(&self) -> bool {
        self.store.current().tinted_frame
    }

    #[zbus(property)]
    fn set_tinted_frame(&self, tinted: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::TintedFrame(tinted))
    }

    #[zbus(property)]
    fn reopen_apps(&self) -> bool {
        self.store.current().reopen_apps
    }

    #[zbus(property)]
    fn set_reopen_apps(&self, on: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::ReopenApps(on))
    }

    #[zbus(property)]
    fn reopen_apps_after_crash(&self) -> bool {
        self.store.current().reopen_apps_after_crash
    }

    #[zbus(property)]
    fn set_reopen_apps_after_crash(&self, on: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::ReopenAppsAfterCrash(on))
    }

    #[zbus(property)]
    fn clock_format(&self) -> String {
        self.store.current().clock.keyword().to_owned()
    }

    #[zbus(property)]
    fn set_clock_format(&self, value: String) -> zbus::fdo::Result<()> {
        let format = keyword(&ClockFormat::ALL, ClockFormat::keyword, &value)?;
        self.write(&Setting::Clock(format))
    }

    #[zbus(property)]
    fn show_battery_percentage(&self) -> bool {
        self.store.current().power.show_battery_percentage
    }

    #[zbus(property)]
    fn set_show_battery_percentage(&self, on: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::ShowBatteryPercentage(on))
    }

    #[zbus(property)]
    fn low_power_on_battery(&self) -> bool {
        self.store.current().power.low_power_on_battery
    }

    #[zbus(property)]
    fn set_low_power_on_battery(&self, on: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::LowPowerOnBattery(on))
    }

    #[zbus(property)]
    fn lid_on_power(&self) -> String {
        self.store.current().power.lid_on_power.keyword().to_owned()
    }

    #[zbus(property)]
    fn set_lid_on_power(&self, value: String) -> zbus::fdo::Result<()> {
        let action = keyword(&LidOnPower::ALL, LidOnPower::keyword, &value)?;
        self.write(&Setting::LidOnPower(action))
    }

    #[zbus(property)]
    fn screen_reader(&self) -> bool {
        self.store.current().accessibility.screen_reader
    }

    #[zbus(property)]
    fn set_screen_reader(&self, on: bool) -> zbus::fdo::Result<()> {
        self.change_accessibility(|access| access.screen_reader = on)
    }

    #[zbus(property)]
    fn zoom(&self) -> f64 {
        self.store.current().accessibility.zoom
    }

    #[zbus(property)]
    fn set_zoom(&self, zoom: f64) -> zbus::fdo::Result<()> {
        let zoom = ZOOM_RANGE.0.max(ZOOM_RANGE.1.min(zoom));
        self.change_accessibility(|access| access.zoom = zoom)
    }

    #[zbus(property)]
    fn increase_contrast(&self) -> bool {
        self.store.current().accessibility.increase_contrast
    }

    #[zbus(property)]
    fn set_increase_contrast(&self, on: bool) -> zbus::fdo::Result<()> {
        self.change_accessibility(|access| access.increase_contrast = on)
    }

    #[zbus(property)]
    fn reduce_transparency(&self) -> bool {
        self.store.current().accessibility.reduce_transparency
    }

    #[zbus(property)]
    fn set_reduce_transparency(&self, on: bool) -> zbus::fdo::Result<()> {
        self.change_accessibility(|access| access.reduce_transparency = on)
    }

    #[zbus(property)]
    fn reduce_motion(&self) -> bool {
        self.store.current().accessibility.reduce_motion
    }

    #[zbus(property)]
    fn set_reduce_motion(&self, on: bool) -> zbus::fdo::Result<()> {
        self.change_accessibility(|access| access.reduce_motion = on)
    }

    #[zbus(property)]
    fn text_size(&self) -> u32 {
        self.store.current().accessibility.text_size
    }

    #[zbus(property)]
    fn set_text_size(&self, percent: u32) -> zbus::fdo::Result<()> {
        let percent = percent.clamp(TEXT_SIZE_RANGE.0, TEXT_SIZE_RANGE.1);
        self.change_accessibility(|access| access.text_size = percent)
    }

    #[zbus(property)]
    fn cursor_size(&self) -> u32 {
        self.store.current().accessibility.cursor_size
    }

    #[zbus(property)]
    fn set_cursor_size(&self, size: u32) -> zbus::fdo::Result<()> {
        let size = size.clamp(CURSOR_SIZE_RANGE.0, CURSOR_SIZE_RANGE.1);
        self.change_accessibility(|access| access.cursor_size = size)
    }

    #[zbus(property)]
    fn flash_screen(&self) -> bool {
        self.store.current().accessibility.flash_screen
    }

    #[zbus(property)]
    fn set_flash_screen(&self, on: bool) -> zbus::fdo::Result<()> {
        self.change_accessibility(|access| access.flash_screen = on)
    }

    #[zbus(property)]
    fn sticky_keys(&self) -> bool {
        self.store.current().accessibility.sticky_keys
    }

    #[zbus(property)]
    fn set_sticky_keys(&self, on: bool) -> zbus::fdo::Result<()> {
        self.change_accessibility(|access| access.sticky_keys = on)
    }

    #[zbus(property)]
    fn slow_keys(&self) -> bool {
        self.store.current().accessibility.slow_keys
    }

    #[zbus(property)]
    fn set_slow_keys(&self, on: bool) -> zbus::fdo::Result<()> {
        self.change_accessibility(|access| access.slow_keys = on)
    }

    #[zbus(property)]
    fn bounce_keys(&self) -> bool {
        self.store.current().accessibility.bounce_keys
    }

    #[zbus(property)]
    fn set_bounce_keys(&self, on: bool) -> zbus::fdo::Result<()> {
        self.change_accessibility(|access| access.bounce_keys = on)
    }

    #[zbus(property)]
    fn bar_apps(&self) -> Vec<String> {
        self.store.current().bar.apps.clone()
    }

    #[zbus(property)]
    fn set_bar_apps(&self, apps: Vec<String>) -> zbus::fdo::Result<()> {
        self.write(&Setting::BarApps(apps))
    }

    #[zbus(property)]
    fn bar_auto_hide(&self) -> bool {
        self.store.current().bar.auto_hide
    }

    #[zbus(property)]
    fn set_bar_auto_hide(&self, auto_hide: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::BarAutoHide(auto_hide))
    }

    #[zbus(property)]
    fn bar_icon_size(&self) -> u32 {
        self.store.current().bar.icon_size
    }

    #[zbus(property)]
    fn set_bar_icon_size(&self, size: u32) -> zbus::fdo::Result<()> {
        self.write(&Setting::BarIconSize(size))
    }

    #[zbus(property)]
    fn bar_edge(&self) -> String {
        self.store.current().bar.edge.keyword().to_owned()
    }

    #[zbus(property)]
    fn set_bar_edge(&self, value: String) -> zbus::fdo::Result<()> {
        let edge = keyword(&BarEdge::ALL, BarEdge::keyword, &value)?;
        let layout = self.store.current().bar.layout;
        let (edge, layout) = bar_placement(edge, layout, false);
        self.write(&Setting::BarPlacement(edge, layout))
    }

    #[zbus(property)]
    fn bar_layout(&self) -> String {
        self.store.current().bar.layout.keyword().to_owned()
    }

    #[zbus(property)]
    fn set_bar_layout(&self, value: String) -> zbus::fdo::Result<()> {
        let layout = keyword(&BarLayout::ALL, BarLayout::keyword, &value)?;
        let edge = self.store.current().bar.edge;
        let (edge, layout) = bar_placement(edge, layout, true);
        self.write(&Setting::BarPlacement(edge, layout))
    }

    #[zbus(property)]
    fn idle_dim_after(&self) -> u64 {
        self.store.current().idle.dim_after.as_secs()
    }

    #[zbus(property)]
    fn set_idle_dim_after(&self, seconds: u64) -> zbus::fdo::Result<()> {
        self.write(&Setting::IdleDimAfter(Duration::from_secs(seconds)))
    }

    #[zbus(property)]
    fn idle_lock_after(&self) -> u64 {
        self.store.current().idle.lock_after.as_secs()
    }

    #[zbus(property)]
    async fn set_idle_lock_after(
        &self,
        seconds: u64,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let before = self.store.current().idle.suspend_after();
        self.write(&Setting::IdleLockAfter(Duration::from_secs(seconds)))?;
        if before != self.store.current().idle.suspend_after() {
            self.idle_suspend_after_changed(&emitter).await?;
        }
        Ok(())
    }

    #[zbus(property)]
    fn idle_suspend_after(&self) -> u64 {
        self.store
            .current()
            .idle
            .suspend_after()
            .map_or(0, |after| after.as_secs())
    }

    #[zbus(property)]
    async fn set_idle_suspend_after(
        &self,
        seconds: u64,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let suspend = match seconds {
            0 => Suspend::Never,
            seconds => Suspend::After(Duration::from_secs(seconds)),
        };
        let before = self.idle_suspend_with_display();
        self.write(&Setting::IdleSuspend(suspend))?;
        if before != self.idle_suspend_with_display() {
            self.idle_suspend_with_display_changed(&emitter).await?;
        }
        Ok(())
    }

    #[zbus(property)]
    fn idle_suspend_with_display(&self) -> bool {
        self.store.current().idle.suspend == Suspend::WithDisplayOff
    }

    #[zbus(property)]
    async fn set_idle_suspend_with_display(
        &self,
        with_display: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let idle = self.store.current().idle;
        let suspend = match (with_display, idle.suspend) {
            (true, _) => Suspend::WithDisplayOff,
            (false, Suspend::WithDisplayOff) => Suspend::After(idle.lock_after),
            (false, explicit) => explicit,
        };
        self.write(&Setting::IdleSuspend(suspend))?;
        if idle.suspend_after() != self.store.current().idle.suspend_after() {
            self.idle_suspend_after_changed(&emitter).await?;
        }
        Ok(())
    }

    #[zbus(property)]
    fn key_repeat_rate(&self) -> u32 {
        self.store.current().keyboard.repeat_rate
    }

    #[zbus(property)]
    fn set_key_repeat_rate(&self, rate: u32) -> zbus::fdo::Result<()> {
        self.write(&Setting::KeyRepeatRate(rate))
    }

    #[zbus(property)]
    fn key_repeat_delay(&self) -> u32 {
        u32::try_from(self.store.current().keyboard.repeat_delay.as_millis()).unwrap_or(u32::MAX)
    }

    #[zbus(property)]
    fn set_key_repeat_delay(&self, milliseconds: u32) -> zbus::fdo::Result<()> {
        self.write(&Setting::KeyRepeatDelay(Duration::from_millis(
            milliseconds.into(),
        )))
    }

    #[zbus(property)]
    fn input_sources(&self) -> Vec<String> {
        self.store
            .current()
            .keyboard
            .input_sources_or(read_system_input_sources)
            .iter()
            .map(InputSource::code)
            .collect()
    }

    #[zbus(property)]
    fn set_input_sources(&self, codes: Vec<String>) -> zbus::fdo::Result<()> {
        let sources = codes
            .iter()
            .map(|code| {
                InputSource::parse(code)
                    .ok_or_else(|| InvalidArgs(format!("\"{code}\" is not a keyboard layout")))
            })
            .collect::<zbus::fdo::Result<Vec<_>>>()?;
        if sources.is_empty() {
            return Err(InvalidArgs(
                "at least one input source is needed".to_owned(),
            ));
        }
        self.write(&Setting::InputSources(sources))
    }

    #[zbus(property)]
    fn caps_lock(&self) -> String {
        self.store.current().keyboard.caps_lock.keyword().to_owned()
    }

    #[zbus(property)]
    fn set_caps_lock(&self, value: String) -> zbus::fdo::Result<()> {
        let caps_lock = keyword(&CapsLock::ALL, CapsLock::keyword, &value)?;
        self.write(&Setting::CapsLock(caps_lock))
    }

    #[zbus(property)]
    fn trackpad_tap_to_click(&self) -> bool {
        self.store.current().trackpad.tap_to_click
    }

    #[zbus(property)]
    fn set_trackpad_tap_to_click(&self, on: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::TrackpadTapToClick(on))
    }

    #[zbus(property)]
    fn trackpad_natural_scrolling(&self) -> bool {
        self.store.current().trackpad.natural_scrolling
    }

    #[zbus(property)]
    fn set_trackpad_natural_scrolling(&self, on: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::TrackpadNaturalScrolling(on))
    }

    #[zbus(property)]
    fn trackpad_speed(&self) -> f64 {
        self.store.current().trackpad.speed
    }

    #[zbus(property)]
    fn set_trackpad_speed(&self, speed: f64) -> zbus::fdo::Result<()> {
        self.write(&Setting::TrackpadSpeed(speed))
    }

    #[zbus(property)]
    fn trackpad_secondary_click(&self) -> String {
        self.store
            .current()
            .trackpad
            .secondary_click
            .keyword()
            .to_owned()
    }

    #[zbus(property)]
    fn set_trackpad_secondary_click(&self, value: String) -> zbus::fdo::Result<()> {
        let click = keyword(
            &[SecondaryClick::TwoFingers, SecondaryClick::Corner],
            SecondaryClick::keyword,
            &value,
        )?;
        self.write(&Setting::TrackpadSecondaryClick(click))
    }

    #[zbus(property)]
    fn trackpad_disable_while_typing(&self) -> bool {
        self.store.current().trackpad.disable_while_typing
    }

    #[zbus(property)]
    fn set_trackpad_disable_while_typing(&self, on: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::TrackpadDisableWhileTyping(on))
    }

    #[zbus(property)]
    fn trackpad_workspace_swipe(&self) -> bool {
        self.store.current().trackpad.workspace_swipe
    }

    #[zbus(property)]
    fn set_trackpad_workspace_swipe(&self, on: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::TrackpadWorkspaceSwipe(on))
    }

    #[zbus(property)]
    fn trackpad_overview_swipe(&self) -> bool {
        self.store.current().trackpad.overview_swipe
    }

    #[zbus(property)]
    fn set_trackpad_overview_swipe(&self, on: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::TrackpadOverviewSwipe(on))
    }

    #[zbus(property)]
    fn trackpad_desktop_spread(&self) -> bool {
        self.store.current().trackpad.desktop_spread
    }

    #[zbus(property)]
    fn set_trackpad_desktop_spread(&self, on: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::TrackpadDesktopSpread(on))
    }

    #[zbus(property)]
    fn trackpad_launcher_pinch(&self) -> bool {
        self.store.current().trackpad.launcher_pinch
    }

    #[zbus(property)]
    fn set_trackpad_launcher_pinch(&self, on: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::TrackpadLauncherPinch(on))
    }

    #[zbus(property)]
    fn trackpad_acceleration(&self) -> bool {
        self.store.current().trackpad.acceleration
    }

    #[zbus(property)]
    fn set_trackpad_acceleration(&self, on: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::TrackpadAcceleration(on))
    }

    #[zbus(property)]
    fn trackpad_dragging(&self) -> String {
        self.store.current().trackpad.dragging.keyword().to_owned()
    }

    #[zbus(property)]
    fn set_trackpad_dragging(&self, value: String) -> zbus::fdo::Result<()> {
        let dragging = keyword(&Dragging::ALL, Dragging::keyword, &value)?;
        self.write(&Setting::TrackpadDragging(dragging))
    }

    #[zbus(property)]
    fn trackpad_scroll_method(&self) -> String {
        self.store
            .current()
            .trackpad
            .scroll_method
            .keyword()
            .to_owned()
    }

    #[zbus(property)]
    fn set_trackpad_scroll_method(&self, value: String) -> zbus::fdo::Result<()> {
        let method = keyword(&ScrollMethod::ALL, ScrollMethod::keyword, &value)?;
        self.write(&Setting::TrackpadScrollMethod(method))
    }

    #[zbus(property)]
    fn mouse_primary_button(&self) -> String {
        self.store
            .current()
            .mouse
            .primary_button
            .keyword()
            .to_owned()
    }

    #[zbus(property)]
    fn set_mouse_primary_button(&self, value: String) -> zbus::fdo::Result<()> {
        let button = keyword(&PrimaryButton::ALL, PrimaryButton::keyword, &value)?;
        self.write(&Setting::MousePrimaryButton(button))
    }

    #[zbus(property)]
    fn mouse_acceleration(&self) -> bool {
        self.store.current().mouse.acceleration
    }

    #[zbus(property)]
    fn set_mouse_acceleration(&self, on: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::MouseAcceleration(on))
    }

    #[zbus(property)]
    fn mouse_natural_scrolling(&self) -> bool {
        self.store.current().mouse.natural_scrolling
    }

    #[zbus(property)]
    fn set_mouse_natural_scrolling(&self, on: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::MouseNaturalScrolling(on))
    }

    #[zbus(property)]
    fn sound_effects(&self) -> bool {
        self.store.current().sound.effects
    }

    #[zbus(property)]
    fn set_sound_effects(&self, on: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::SoundEffects(on))
    }

    #[zbus(property)]
    fn volume_feedback(&self) -> bool {
        self.store.current().sound.volume_feedback
    }

    #[zbus(property)]
    fn set_volume_feedback(&self, on: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::VolumeFeedback(on))
    }

    #[zbus(property)]
    fn alert_sound(&self) -> String {
        self.store.current().sound.alert.keyword().to_owned()
    }

    #[zbus(property)]
    fn set_alert_sound(&self, value: String) -> zbus::fdo::Result<()> {
        let alert = keyword(&AlertSound::ALL, AlertSound::keyword, &value)?;
        self.write(&Setting::AlertSound(alert))
    }

    #[zbus(property)]
    fn alert_volume(&self) -> f64 {
        self.store.current().sound.alert_volume
    }

    #[zbus(property)]
    fn set_alert_volume(&self, volume: f64) -> zbus::fdo::Result<()> {
        self.write(&Setting::AlertVolume(volume))
    }

    #[zbus(property)]
    fn mouse_speed(&self) -> f64 {
        self.store.current().mouse.speed
    }

    #[zbus(property)]
    fn set_mouse_speed(&self, speed: f64) -> zbus::fdo::Result<()> {
        self.write(&Setting::MouseSpeed(speed))
    }

    #[zbus(property)]
    fn wallpaper_light(&self) -> String {
        self.wallpaper(WallpaperVariant::Light)
    }

    #[zbus(property)]
    fn set_wallpaper_light(&self, path: String) -> zbus::fdo::Result<()> {
        self.set_wallpaper(WallpaperVariant::Light, path)
    }

    #[zbus(property)]
    fn wallpaper_dark(&self) -> String {
        self.wallpaper(WallpaperVariant::Dark)
    }

    #[zbus(property)]
    fn set_wallpaper_dark(&self, path: String) -> zbus::fdo::Result<()> {
        self.set_wallpaper(WallpaperVariant::Dark, path)
    }

    #[zbus(property)]
    fn wallpaper_fit(&self) -> String {
        self.store.current().wallpaper.fit.keyword().to_owned()
    }

    #[zbus(property)]
    fn set_wallpaper_fit(&self, value: String) -> zbus::fdo::Result<()> {
        let fit = keyword(
            &[WallpaperFit::Fill, WallpaperFit::Fit],
            WallpaperFit::keyword,
            &value,
        )?;
        self.write(&Setting::WallpaperFit(fit))
    }

    #[zbus(property)]
    fn focus_schedule(&self) -> FocusScheduleValue {
        let schedule = self.store.current().focus_schedule;
        (schedule.on, schedule.from.text(), schedule.to.text())
    }

    #[zbus(property)]
    fn set_focus_schedule(&self, value: FocusScheduleValue) -> zbus::fdo::Result<()> {
        let (on, from, to) = value;
        let schedule = FocusSchedule {
            on,
            from: clock_time(&from)?,
            to: clock_time(&to)?,
        };
        self.write(&Setting::FocusSchedule(schedule))
    }

    #[zbus(property)]
    fn night_light(&self) -> NightLightValue {
        let night = self.store.current().night_light;
        (
            night.on,
            night.schedule.keyword().to_owned(),
            night.from.text(),
            night.to.text(),
            night.warmth,
        )
    }

    #[zbus(property)]
    fn set_night_light(&self, value: NightLightValue) -> zbus::fdo::Result<()> {
        let (on, schedule, from, to, warmth) = value;
        let time = clock_time;
        let (least, most) = NIGHT_LIGHT_WARMTH_RANGE;
        if !(least..=most).contains(&warmth) {
            return Err(InvalidArgs(format!(
                "the warmth must lie between {least} and {most}"
            )));
        }
        let night = NightLight {
            on,
            schedule: keyword(
                &[NightSchedule::Sunset, NightSchedule::Custom],
                NightSchedule::keyword,
                &schedule,
            )?,
            from: time(&from)?,
            to: time(&to)?,
            warmth,
        };
        self.write(&Setting::NightLight(night))
    }

    #[zbus(property)]
    fn search_shows(&self) -> HashMap<String, bool> {
        let search = &self.store.current().search;
        SearchCategory::ALL
            .into_iter()
            .map(|category| (category.keyword().to_owned(), search.shows(category)))
            .collect()
    }

    #[zbus(property)]
    fn set_search_shows(&self, shows: HashMap<String, bool>) -> zbus::fdo::Result<()> {
        let changes = shows
            .into_iter()
            .map(|(keyword, shown)| {
                SearchCategory::from_keyword(&keyword)
                    .map(|category| (category, shown))
                    .ok_or_else(|| InvalidArgs(format!("unknown search category `{keyword}`")))
            })
            .collect::<zbus::fdo::Result<Vec<_>>>()?;
        self.write_from(|config| {
            let mut search = config.search.clone();
            for (category, shown) in changes {
                search.set_shown(category, shown);
            }
            Setting::Search(search)
        })
    }

    #[zbus(property)]
    fn search_excluded(&self) -> Vec<String> {
        self.store
            .current()
            .search
            .exclude
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    }

    #[zbus(property)]
    fn set_search_excluded(&self, paths: Vec<String>) -> zbus::fdo::Result<()> {
        if let Some(path) = paths.iter().find(|path| !is_wallpaper_path(path)) {
            return Err(InvalidArgs(format!(
                "\"{path}\" is not an absolute path or one starting with \"~/\""
            )));
        }
        self.write_from(|config| {
            let mut search = config.search.clone();
            search.exclude = paths.into_iter().map(PathBuf::from).collect();
            Setting::Search(search)
        })
    }

    #[zbus(property)]
    fn displays(&self) -> HashMap<String, HashMap<String, OwnedValue>> {
        let owned = |value: Value<'_>| OwnedValue::try_from(value).ok();
        self.store
            .current()
            .displays
            .iter()
            .map(|(identity, display)| {
                let settings = [
                    (
                        "resolution",
                        display.resolution.and_then(|size| owned(size.into())),
                    ),
                    ("scale", display.scale.and_then(|scale| owned(scale.into()))),
                    (
                        "refresh-rate",
                        display.refresh_rate.and_then(|rate| owned(rate.into())),
                    ),
                    (
                        "rotation",
                        display
                            .rotation
                            .and_then(|rotation| owned(rotation.degrees().into())),
                    ),
                    (
                        "variable-refresh-rate",
                        display
                            .variable_refresh_rate
                            .and_then(|enabled| owned(enabled.into())),
                    ),
                ]
                .into_iter()
                .filter_map(|(key, value)| Some((key.to_owned(), value?)))
                .collect();
                (identity.clone(), settings)
            })
            .collect()
    }

    async fn set_display(
        &self,
        identity: String,
        settings: HashMap<String, OwnedValue>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let before = self.store.current();
        self.write(&Setting::Display(identity, display_settings(&settings)?))?;
        if before.displays != self.store.current().displays {
            self.displays_changed(&emitter).await?;
        }
        Ok(())
    }

    #[zbus(property)]
    fn arrangements(&self) -> Vec<Vec<ArrangedDisplay>> {
        self.store
            .current()
            .arrangements
            .iter()
            .map(|placements| placements.iter().map(arranged).collect())
            .collect()
    }

    async fn set_arrangement(
        &self,
        displays: Vec<ArrangedDisplay>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let before = self.store.current();
        let placements = displays.into_iter().map(placement).collect();
        self.write(&Setting::Arrangement(placements))?;
        if before.arrangements != self.store.current().arrangements {
            self.arrangements_changed(&emitter).await?;
        }
        Ok(())
    }
}
