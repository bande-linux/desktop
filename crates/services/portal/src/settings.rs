use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use bande_config::{ACCENTS, Accent};
use bande_design::{Color, Scheme, Typeface};
use bande_wallpaper::{Choice, Variant};
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use rustix::io::{Errno, read};
use rustix::time::{
    Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, timerfd_create,
    timerfd_settime,
};
use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;
use zbus::object_server::SignalEmitter;
use zbus::proxy::CacheProperties;
use zbus::zvariant::{OwnedValue, Value};

use crate::{OBJECT_PATH, gtk};
use bande_datetime::daylight::{Sky, place_of, sky};
use bande_datetime::local_zone;
use bande_datetime::timedate::zone_changes;

pub const APPEARANCE: &str = "org.freedesktop.appearance";
pub const BANDE_APPEARANCE: &str = "org.bande.appearance";
pub const GNOME_INTERFACE: &str = "org.gnome.desktop.interface";
pub const GNOME_SOUND: &str = "org.gnome.desktop.sound";
pub const SOUND_THEME: &str = "Bande";
pub const ICON_THEME: &str = "Bande";
pub const VERSION: u32 = 2;

const CONFIG_BUS_NAME: &str = "org.bande.Config";
const CONFIG_PATH: &str = "/org/bande/Config";
const TINT_SAMPLE: u32 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorScheme {
    PreferDark = 1,
    PreferLight = 2,
}

impl ColorScheme {
    fn scheme(self) -> Scheme {
        match self {
            Self::PreferDark => Scheme::Dark,
            Self::PreferLight => Scheme::Light,
        }
    }

    fn gnome_color_scheme(self) -> &'static str {
        match self {
            Self::PreferDark => "prefer-dark",
            Self::PreferLight => "prefer-light",
        }
    }

    fn gtk_theme(self, increase_contrast: bool) -> &'static str {
        match (self, increase_contrast) {
            (Self::PreferLight, false) => "Adwaita",
            (Self::PreferDark, false) => "Adwaita-dark",
            (Self::PreferLight, true) => "HighContrast",
            (Self::PreferDark, true) => "HighContrastInverse",
        }
    }
}

impl From<Scheme> for ColorScheme {
    fn from(scheme: Scheme) -> Self {
        match scheme {
            Scheme::Dark => Self::PreferDark,
            Scheme::Light => Self::PreferLight,
        }
    }
}

fn font(typeface: Typeface, points: u32) -> String {
    format!("{} {points}", typeface.family())
}

fn fractions(color: Color) -> (f64, f64, f64) {
    let channel = |value: u8| f64::from(value) / 255.0;
    (channel(color.r), channel(color.g), channel(color.b))
}

pub fn icon_theme(accent: Accent) -> String {
    let keyword = ACCENTS
        .iter()
        .find(|(_, candidate)| *candidate == accent)
        .map_or("", |(keyword, _)| keyword);
    let mut letters = keyword.chars();
    match (accent, letters.next()) {
        (Accent::Ultramarine, _) | (_, None) => ICON_THEME.to_owned(),
        (_, Some(first)) => format!("{ICON_THEME}-{}{}", first.to_uppercase(), letters.as_str()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorSettings {
    pub theme: String,
}

impl CursorSettings {
    pub fn from_environment() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        Self {
            theme: var("XCURSOR_THEME").unwrap_or_else(|| "default".to_owned()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Look {
    scheme: ColorScheme,
    accent: Accent,
    tint: Option<Color>,
    increase_contrast: bool,
    reduce_transparency: bool,
    reduce_motion: bool,
    text_size: u32,
    cursor_size: u32,
    effects: bool,
}

#[zbus::proxy(
    interface = "org.bande.Config",
    default_service = "org.bande.Config",
    default_path = "/org/bande/Config"
)]
trait Config {
    #[zbus(property)]
    fn appearance(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn accent(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn tinted_frame(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn increase_contrast(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn reduce_transparency(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn reduce_motion(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn text_size(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn cursor_size(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn sound_effects(&self) -> zbus::Result<bool>;
}

pub fn average(rgba: &[u8]) -> Option<Color> {
    let pixels = rgba.as_chunks::<4>().0;
    let count = u64::try_from(pixels.len())
        .ok()
        .filter(|count| *count > 0)?;
    let mut sums = [0u64; 3];
    for pixel in pixels {
        for (sum, channel) in sums.iter_mut().zip(pixel) {
            *sum += u64::from(*channel);
        }
    }
    let [r, g, b] = sums.map(|sum| u8::try_from(sum / count).unwrap_or(u8::MAX));
    Some(Color {
        r,
        g,
        b,
        a: u8::MAX,
    })
}

fn data_dirs() -> Vec<PathBuf> {
    std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned())
        .split(':')
        .map(PathBuf::from)
        .collect()
}

#[derive(Default)]
struct TintCache {
    picture: Option<(PathBuf, Option<Color>)>,
}

impl TintCache {
    fn tint(&mut self, choice: &Choice, scheme: Scheme) -> Option<Color> {
        let variant = match scheme {
            Scheme::Light => Variant::Light,
            Scheme::Dark => Variant::Dark,
        };
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let path = choice.path(variant, home.as_deref(), &data_dirs())?;
        if let Some((cached, tint)) = &self.picture
            && *cached == path
        {
            return *tint;
        }
        let tint = bande_wallpaper::render(&path, choice.fit, TINT_SAMPLE, TINT_SAMPLE)
            .map_err(|error| tracing::warn!("cannot tint the frame from the wallpaper: {error}"))
            .ok()
            .and_then(|picture| average(&picture.rgba));
        self.picture = Some((path, tint));
        tint
    }
}

struct Reader {
    connection: Connection,
    config: ConfigProxyBlocking<'static>,
    tints: TintCache,
}

impl Reader {
    fn new(connection: &Connection) -> zbus::Result<Self> {
        Ok(Self {
            connection: connection.clone(),
            config: ConfigProxyBlocking::builder(connection)
                .cache_properties(CacheProperties::No)
                .build()?,
            tints: TintCache::default(),
        })
    }

    fn read(&mut self, now: &Zoned) -> (Look, Option<Timestamp>) {
        let config = &self.config;
        let (scheme, next) = match config.appearance().unwrap_or_default().as_str() {
            "dark" => (Scheme::Dark, None),
            "light" => (Scheme::Light, None),
            _ => {
                sky(now, place_of(now.time_zone())).map_or((Scheme::Light, None), |(sky, next)| {
                    let scheme = match sky {
                        Sky::Day => Scheme::Light,
                        Sky::Night => Scheme::Dark,
                    };
                    (scheme, Some(next))
                })
            }
        };
        let accent = config.accent().unwrap_or_default();
        let tint = config
            .tinted_frame()
            .unwrap_or(false)
            .then(|| {
                let choice = bande_wallpaper::read(&self.connection);
                self.tints.tint(&choice, scheme)
            })
            .flatten();
        let look = Look {
            scheme: scheme.into(),
            accent: ACCENTS
                .iter()
                .find(|(keyword, _)| *keyword == accent)
                .map_or_else(Accent::default, |(_, accent)| *accent),
            tint,
            increase_contrast: config.increase_contrast().unwrap_or(false),
            reduce_transparency: config.reduce_transparency().unwrap_or(false),
            reduce_motion: config.reduce_motion().unwrap_or(false),
            text_size: config.text_size().unwrap_or(100),
            cursor_size: config.cursor_size().unwrap_or(24),
            effects: config.sound_effects().unwrap_or(true),
        };
        (look, next)
    }
}

type Namespaces = Vec<(&'static str, Vec<(&'static str, OwnedValue)>)>;

fn namespaces(look: Look, cursor: &CursorSettings) -> Namespaces {
    let owned = |values: Vec<(&'static str, Value<'_>)>| {
        values
            .into_iter()
            .filter_map(|(key, value)| Some((key, OwnedValue::try_from(value).ok()?)))
            .collect()
    };
    let tint: Vec<f64> = look
        .tint
        .map(|tint| {
            let (r, g, b) = fractions(tint);
            vec![r, g, b]
        })
        .unwrap_or_default();
    vec![
        (
            APPEARANCE,
            owned(vec![
                ("color-scheme", Value::from(look.scheme as u32)),
                (
                    "accent-color",
                    Value::from(fractions(look.accent.color(look.scheme.scheme()))),
                ),
                ("contrast", Value::from(u32::from(look.increase_contrast))),
                ("reduced-motion", Value::from(u32::from(look.reduce_motion))),
            ]),
        ),
        (
            BANDE_APPEARANCE,
            owned(vec![
                ("frame-tint", Value::from(tint)),
                (
                    "reduce-transparency",
                    Value::from(u32::from(look.reduce_transparency)),
                ),
                ("text-size", Value::from(look.text_size)),
            ]),
        ),
        (
            GNOME_INTERFACE,
            owned(vec![
                (
                    "color-scheme",
                    Value::from(look.scheme.gnome_color_scheme()),
                ),
                (
                    "gtk-theme",
                    Value::from(look.scheme.gtk_theme(look.increase_contrast)),
                ),
                ("font-name", Value::from(font(Typeface::Interface, 10))),
                (
                    "document-font-name",
                    Value::from(font(Typeface::Interface, 11)),
                ),
                (
                    "monospace-font-name",
                    Value::from(font(Typeface::Monospace, 10)),
                ),
                ("enable-animations", Value::from(!look.reduce_motion)),
                ("icon-theme", Value::from(icon_theme(look.accent))),
                ("cursor-theme", Value::from(cursor.theme.as_str())),
                (
                    "cursor-size",
                    Value::from(i32::try_from(look.cursor_size).unwrap_or(i32::MAX)),
                ),
                (
                    "text-scaling-factor",
                    Value::from(f64::from(look.text_size) / 100.0),
                ),
            ]),
        ),
        (
            GNOME_SOUND,
            owned(vec![
                ("theme-name", Value::from(SOUND_THEME)),
                ("event-sounds", Value::from(look.effects)),
            ]),
        ),
    ]
}

fn matches(namespace: &str, filters: &[String]) -> bool {
    filters.is_empty()
        || filters.iter().any(|filter| match filter.strip_suffix('*') {
            Some(prefix) => namespace.starts_with(prefix),
            None => filter == namespace,
        })
}

fn signal_changes(connection: &Connection, old: Look, new: Look, cursor: &CursorSettings) {
    let Ok(emitter) = SignalEmitter::new(connection.inner(), OBJECT_PATH) else {
        return;
    };
    let before: HashMap<_, _> = namespaces(old, cursor)
        .into_iter()
        .flat_map(|(namespace, values)| {
            values
                .into_iter()
                .map(move |(key, value)| ((namespace, key), value))
        })
        .collect();
    for (namespace, values) in namespaces(new, cursor) {
        for (key, value) in values {
            if before.get(&(namespace, key)) != Some(&value) {
                let _ = zbus::block_on(Settings::setting_changed(
                    &emitter,
                    namespace,
                    key,
                    Value::from(value),
                ));
            }
        }
    }
}

const DISARMED: Timespec = Timespec {
    tv_sec: 0,
    tv_nsec: 0,
};

fn wake_at(timer: &OwnedFd, next: Option<Timestamp>) {
    let it_value = next.map_or(DISARMED, |next| Timespec {
        tv_sec: next.as_second().max(1),
        tv_nsec: 0,
    });
    let armed = timerfd_settime(
        timer,
        TimerfdTimerFlags::ABSTIME | TimerfdTimerFlags::CANCEL_ON_SET,
        &Itimerspec {
            it_interval: DISARMED,
            it_value,
        },
    );
    if let Err(error) = armed {
        tracing::warn!("cannot wait for the next change of appearance: {error}");
    }
}

fn follow_timer(timer: Arc<OwnedFd>, wake: mpsc::Sender<Option<TimeZone>>) {
    let mut expirations = [0; 8];
    loop {
        match read(&*timer, &mut expirations) {
            Ok(_) | Err(Errno::CANCELED) => {}
            Err(Errno::INTR) => continue,
            Err(error) => {
                tracing::warn!("the automatic appearance stopped following the time: {error}");
                return;
            }
        }
        if wake.send(None).is_err() {
            return;
        }
    }
}

fn follow_zone(system: &Connection, wake: mpsc::Sender<Option<TimeZone>>) -> zbus::Result<()> {
    let zones = zone_changes(system)?;
    thread::spawn(move || {
        for zone in zones {
            if wake.send(TimeZone::get(&zone).ok()).is_err() {
                return;
            }
        }
    });
    Ok(())
}

pub struct Settings {
    look: Arc<Mutex<Look>>,
    cursor: CursorSettings,
}

impl Settings {
    pub fn serve(
        connection: &Connection,
        system: Option<&Connection>,
        cursor: CursorSettings,
        gtk_home: Option<PathBuf>,
    ) -> zbus::Result<()> {
        let changes = PropertiesProxy::builder(connection)
            .destination(CONFIG_BUS_NAME)?
            .path(CONFIG_PATH)?
            .build()?
            .receive_properties_changed()?;
        let mut reader = Reader::new(connection)?;
        let (first, next) = reader.read(&bande_datetime::now());
        let style_gtk = move |look: Look| {
            if let Some(home) = &gtk_home
                && let Err(error) = gtk::install(home, look.accent, look.scheme.scheme())
            {
                tracing::warn!("cannot give GTK apps the Bande colours: {error}");
            }
        };
        style_gtk(first);
        let look = Arc::new(Mutex::new(first));
        connection.object_server().at(
            OBJECT_PATH,
            Settings {
                look: Arc::clone(&look),
                cursor: cursor.clone(),
            },
        )?;
        let (wake, woken) = mpsc::channel();
        let timer = Arc::new(
            timerfd_create(TimerfdClockId::Realtime, TimerfdFlags::CLOEXEC)
                .map_err(|error| zbus::Error::from(std::io::Error::from(error)))?,
        );
        wake_at(&timer, next);
        let (waiting, timer_wake) = (Arc::clone(&timer), wake.clone());
        thread::spawn(move || follow_timer(waiting, timer_wake));
        if let Some(system) = system
            && let Err(error) = follow_zone(system, wake.clone())
        {
            tracing::warn!("the automatic appearance does not follow time zone changes: {error}");
        }
        thread::spawn(move || {
            for _ in changes {
                if wake.send(None).is_err() {
                    break;
                }
            }
        });
        let connection = connection.clone();
        thread::spawn(move || {
            while let Ok(zone) = woken.recv() {
                let zone = zone.unwrap_or_else(local_zone);
                let (new, next) = reader.read(&Timestamp::now().to_zoned(zone));
                wake_at(&timer, next);
                let old = std::mem::replace(
                    &mut *look.lock().unwrap_or_else(PoisonError::into_inner),
                    new,
                );
                if (old.accent, old.scheme) != (new.accent, new.scheme) {
                    style_gtk(new);
                }
                if old != new {
                    signal_changes(&connection, old, new, &cursor);
                }
            }
        });
        Ok(())
    }

    fn current(&self) -> Namespaces {
        let look = *self.look.lock().unwrap_or_else(PoisonError::into_inner);
        namespaces(look, &self.cursor)
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
    fn read_all(&self, filters: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        self.current()
            .into_iter()
            .filter(|(namespace, _)| matches(namespace, &filters))
            .map(|(namespace, values)| {
                let values = values
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value))
                    .collect();
                (namespace.to_owned(), values)
            })
            .collect()
    }

    fn read(&self, namespace: String, key: String) -> Result<OwnedValue, PortalError> {
        self.current()
            .into_iter()
            .filter(|(name, _)| *name == namespace)
            .flat_map(|(_, values)| values)
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value)
            .ok_or_else(|| PortalError::NotFound(format!("{namespace} {key} is not set")))
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
