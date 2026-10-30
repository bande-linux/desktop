use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use unbound_watch::{Changes, FolderWatch};
use zbus::blocking::Connection;
use zbus::fdo::Error::{Failed, InvalidArgs};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

use crate::{
    ACCENTS, AlertSound, Appearance, BarEdge, CapsLock, ClockFormat, Config, ConfigStore,
    DisplaySettings, InputSource, LidOnPower, Loaded, MenuBarPlacement, Placement, ReloadError,
    Rotation, SecondaryClick, Setting, Suspend, WallpaperFit, WallpaperVariant, accent_keyword,
    migrate, read_system_input_sources,
};

pub type ArrangedDisplay = (String, i32, i32, bool, String);

pub const BUS_NAME: &str = "org.unbound.Config";
pub const OBJECT_PATH: &str = "/org/unbound/Config";

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
    let staging = path.with_file_name(STAGING_NAME);
    std::fs::write(&staging, contents)?;
    std::fs::rename(&staging, path)
}

pub fn migrate_file(path: &Path, connected: &[(String, String)]) -> io::Result<bool> {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    match migrate(&source, connected) {
        Ok(Some(migrated)) => replace(path, &migrated).map(|()| true),
        Ok(None) | Err(_) => Ok(false),
    }
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
        if before.reduce_motion != after.reduce_motion {
            let _ = guard.reduce_motion_changed(emitter).await;
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
        if before.bar.names != after.bar.names {
            let _ = guard.bar_names_changed(emitter).await;
        }
        if before.bar.menu_bar != after.bar.menu_bar {
            let _ = guard.bar_menu_bar_changed(emitter).await;
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
        if before.mouse.natural_scrolling != after.mouse.natural_scrolling {
            let _ = guard.mouse_natural_scrolling_changed(emitter).await;
        }
        if before.mouse.speed != after.mouse.speed {
            let _ = guard.mouse_speed_changed(emitter).await;
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
        let mut published = lock(&self.published);
        let source = match std::fs::read_to_string(&self.path) {
            Ok(source) => source,
            Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(Failed(format!("cannot read the configuration: {error}")));
            }
        };
        if let Err(error) = Config::parse(&source) {
            return Err(Failed(format!(
                "the configuration is invalid at {error} and must be fixed first"
            )));
        }
        let edited = setting
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

#[zbus::interface(name = "org.unbound.Config")]
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
    fn reduce_motion(&self) -> bool {
        self.store.current().reduce_motion
    }

    #[zbus(property)]
    fn set_reduce_motion(&self, reduce: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::ReduceMotion(reduce))
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
    fn bar_names(&self) -> bool {
        self.store.current().bar.names
    }

    #[zbus(property)]
    fn set_bar_names(&self, names: bool) -> zbus::fdo::Result<()> {
        self.write(&Setting::BarNames(names))
    }

    #[zbus(property)]
    fn bar_edge(&self) -> String {
        self.store.current().bar.edge.keyword().to_owned()
    }

    #[zbus(property)]
    fn set_bar_edge(&self, value: String) -> zbus::fdo::Result<()> {
        let edge = keyword(&BarEdge::ALL, BarEdge::keyword, &value)?;
        self.write(&Setting::BarEdge(edge))
    }

    #[zbus(property)]
    fn bar_menu_bar(&self) -> String {
        self.store.current().bar.menu_bar.keyword().to_owned()
    }

    #[zbus(property)]
    fn set_bar_menu_bar(&self, value: String) -> zbus::fdo::Result<()> {
        let placement = keyword(&MenuBarPlacement::ALL, MenuBarPlacement::keyword, &value)?;
        self.write(&Setting::BarMenuBar(placement))
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
