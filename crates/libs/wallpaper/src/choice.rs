use std::path::{Path, PathBuf};
use std::thread;

use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;

use unbound_inbox::Outbox;

pub const DEFAULT_WALLPAPER: &str = "drift";

const CONFIG_BUS_NAME: &str = "org.unbound.Config";
const CONFIG_PATH: &str = "/org/unbound/Config";

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

#[zbus::proxy(
    interface = "org.unbound.Config",
    default_service = "org.unbound.Config",
    default_path = "/org/unbound/Config"
)]
trait Config {
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
}

fn configured(value: zbus::Result<String>) -> Option<PathBuf> {
    value
        .ok()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

fn read_with(config: &ConfigProxyBlocking<'_>) -> Choice {
    Choice {
        light: configured(config.wallpaper_light()),
        dark: configured(config.wallpaper_dark()),
        fit: match config.wallpaper_fit().ok().as_deref() {
            Some("fit") => Fit::Fit,
            _ => Fit::Fill,
        },
    }
}

fn proxy(session: &Connection) -> zbus::Result<ConfigProxyBlocking<'static>> {
    ConfigProxyBlocking::builder(session)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}

pub fn read(session: &Connection) -> Choice {
    proxy(session).map_or_else(|_| Choice::default(), |config| read_with(&config))
}

pub fn follow(choices: Outbox<Choice>) {
    let work = move || {
        let mut sent = false;
        let mut follow = || -> zbus::Result<()> {
            let session = Connection::session()?;
            let config = proxy(&session)?;
            let changes = PropertiesProxy::builder(&session)
                .destination(CONFIG_BUS_NAME)?
                .path(CONFIG_PATH)?
                .build()?
                .receive_properties_changed()?;
            let mut last = read_with(&config);
            sent = true;
            if !choices.send(last.clone()) {
                return Ok(());
            }
            for _ in changes {
                let choice = read_with(&config);
                if choice != last {
                    last = choice.clone();
                    if !choices.send(choice) {
                        break;
                    }
                }
            }
            Ok(())
        };
        if let Err(error) = follow() {
            tracing::warn!("cannot follow the wallpaper in org.unbound.Config: {error}");
            if !sent {
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

pub fn choose(path: PathBuf) {
    let work = move || {
        let choose = || -> zbus::Result<()> {
            let session = Connection::session()?;
            let config = proxy(&session)?;
            let path = path.to_string_lossy();
            config.set_wallpaper_light(&path)?;
            config.set_wallpaper_dark(&path)
        };
        if let Err(error) = choose() {
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
