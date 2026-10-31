use std::path::{Path, PathBuf};
use std::thread;

use zbus::blocking::Connection;

use unbound_bus::{follow_properties, get_all};
use unbound_config::client::{ConfigProxy, INTERFACE, Properties};
use unbound_config::{BUS_NAME, OBJECT_PATH};
use unbound_inbox::Outbox;

pub const DEFAULT_WALLPAPER: &str = "drift";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Variant {
    Light,
    Dark,
}

impl Variant {
    fn keyword(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Fit {
    #[default]
    Fill,
    Fit,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Choice {
    pub light: Option<PathBuf>,
    pub dark: Option<PathBuf>,
    pub fit: Fit,
}

fn expand(path: &Path, home: Option<&Path>) -> Option<PathBuf> {
    match path.strip_prefix("~") {
        Ok(rest) => home.map(|home| home.join(rest)),
        Err(_) => path.is_absolute().then(|| path.to_path_buf()),
    }
}

pub fn bundled(name: &str, variant: Variant, data_dirs: &[PathBuf]) -> Option<PathBuf> {
    data_dirs
        .iter()
        .map(|dir| {
            dir.join("unbound/wallpapers")
                .join(format!("{name}-{}.svg", variant.keyword()))
        })
        .find(|path| path.is_file())
}

impl Choice {
    pub fn path(
        &self,
        variant: Variant,
        home: Option<&Path>,
        data_dirs: &[PathBuf],
    ) -> Option<PathBuf> {
        let (own, other) = match variant {
            Variant::Light => (&self.light, &self.dark),
            Variant::Dark => (&self.dark, &self.light),
        };
        own.iter()
            .chain(other)
            .find_map(|path| expand(path, home))
            .or_else(|| bundled(DEFAULT_WALLPAPER, variant, data_dirs))
    }
}

fn configured(value: &Option<String>) -> Option<PathBuf> {
    value
        .as_deref()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

pub fn choice_of(config: &Properties) -> Choice {
    Choice {
        light: configured(&config.wallpaper_light),
        dark: configured(&config.wallpaper_dark),
        fit: match config.wallpaper_fit.as_deref() {
            Some("fit") => Fit::Fit,
            _ => Fit::Fill,
        },
    }
}

fn proxy(session: &Connection) -> zbus::Result<ConfigProxy<'static>> {
    ConfigProxy::builder(session)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}

pub fn read(session: &Connection) -> Choice {
    get_all(session, BUS_NAME, OBJECT_PATH, INTERFACE)
        .map_or_else(|_| Choice::default(), |config| choice_of(&config))
}

pub fn follow(session: &Connection, choices: Outbox<Choice>) {
    let session = session.clone();
    let work = move || {
        let mut config = Properties::default();
        let mut last = None;
        let followed = follow_properties(&session, BUS_NAME, OBJECT_PATH, INTERFACE, |changed| {
            config.update(changed);
            let choice = choice_of(&config);
            if last.as_ref() == Some(&choice) {
                return true;
            }
            last = Some(choice.clone());
            choices.send(choice)
        });
        if let Err(error) = followed {
            tracing::warn!("cannot follow the wallpaper in org.unbound.Config: {error}");
            if last.is_none() {
                choices.send(Choice::default());
            }
        }
    };
    if let Err(error) = thread::Builder::new()
        .name("wallpaper choice".to_owned())
        .spawn(work)
    {
        tracing::warn!("cannot start following the wallpaper: {error}");
    }
}

pub fn set(session: &Connection, path: &Path) -> zbus::Result<()> {
    let config = proxy(session)?;
    let path = path.to_string_lossy();
    config.set_wallpaper_light(&path)?;
    config.set_wallpaper_dark(&path)
}

pub fn choose(session: &Connection, path: PathBuf) {
    let session = session.clone();
    let work = move || {
        if let Err(error) = set(&session, &path) {
            tracing::warn!("cannot set the wallpaper: {error}");
        }
    };
    if let Err(error) = thread::Builder::new()
        .name("wallpaper choice".to_owned())
        .spawn(work)
    {
        tracing::warn!("cannot start setting the wallpaper: {error}");
    }
}
