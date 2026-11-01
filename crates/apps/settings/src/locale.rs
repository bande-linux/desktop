use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Sender};

use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;

use crate::failure::{failed, unexpected};
use crate::follow::{Update, Updates, spawn};
use bande_settings_panes::SettingId;

const LOCALE: &str = "org.freedesktop.locale1";
const LOCALE_PATH: &str = "/org/freedesktop/locale1";
const LOCALE_SOURCES: &str = "/usr/share/i18n/locales";
pub const REGION_CATEGORIES: [&str; 5] = [
    "LC_TIME",
    "LC_NUMERIC",
    "LC_MONETARY",
    "LC_MEASUREMENT",
    "LC_PAPER",
];

#[zbus::proxy(
    interface = "org.freedesktop.locale1",
    default_service = "org.freedesktop.locale1",
    default_path = "/org/freedesktop/locale1"
)]
trait Locale {
    #[zbus(property)]
    fn locale(&self) -> zbus::Result<Vec<String>>;
    fn set_locale(&self, locale: Vec<String>, interactive: bool) -> zbus::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledLocale {
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocaleState {
    pub installed: Vec<InstalledLocale>,
    pub language: String,
    pub region: Option<String>,
}

impl LocaleState {
    pub fn territory(&self) -> Option<&str> {
        let code = base(self.region.as_deref().unwrap_or(&self.language));
        code.split_once('_').map(|(_, territory)| territory)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocaleCommand {
    Language(String),
    Region(Option<String>),
}

pub fn locale_name(code: &str, source: &str) -> String {
    let field = |name: &str| {
        source.lines().find_map(|line| {
            let value = line.trim().strip_prefix(name)?;
            value
                .starts_with([' ', '\t'])
                .then(|| value.trim().trim_matches('"').to_owned())
        })
    };
    match (field("language"), field("territory")) {
        (Some(language), Some(territory)) if !language.is_empty() => {
            format!("{language} ({territory})")
        }
        _ => code.to_owned(),
    }
}

fn base(code: &str) -> &str {
    code.split(['.', '@']).next().unwrap_or(code)
}

pub fn installed_locales(listing: &str, sources: &Path) -> Vec<InstalledLocale> {
    let mut installed: Vec<InstalledLocale> = listing
        .lines()
        .map(str::trim)
        .filter(|code| {
            base(code).contains('_') && code.to_lowercase().replace('-', "").contains(".utf8")
        })
        .map(|code| InstalledLocale {
            code: format!("{}.UTF-8", base(code)),
            name: locale_name(
                base(code),
                &std::fs::read_to_string(sources.join(base(code))).unwrap_or_default(),
            ),
        })
        .collect();
    installed.sort_by_key(|locale| locale.name.to_lowercase());
    installed.dedup_by(|a, b| a.code == b.code);
    installed
}

pub fn parse_assignments(assignments: &[String]) -> (String, Option<String>) {
    let value = |name: &str| {
        assignments
            .iter()
            .find_map(|assignment| assignment.strip_prefix(name)?.strip_prefix('='))
            .map(str::to_owned)
    };
    (
        value("LANG").unwrap_or_default(),
        value(REGION_CATEGORIES[0]),
    )
}

pub fn assignments(language: &str, region: Option<&str>) -> Vec<String> {
    let mut assignments = vec![format!("LANG={language}")];
    let short = base(language);
    if let Some((tongue, _)) = short.split_once('_') {
        assignments.push(format!("LANGUAGE={short}:{tongue}"));
    }
    if let Some(region) = region.filter(|region| *region != language) {
        assignments.extend(
            REGION_CATEGORIES
                .iter()
                .map(|category| format!("{category}={region}")),
        );
    }
    assignments
}

fn read(locale: &LocaleProxyBlocking<'_>, installed: &[InstalledLocale]) -> Option<LocaleState> {
    let (language, region) = parse_assignments(&locale.locale().ok()?);
    Some(LocaleState {
        installed: installed.to_vec(),
        language,
        region,
    })
}

fn listing() -> String {
    Command::new("locale")
        .arg("-a")
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}

pub fn follow_locale(system: &Connection, updates: Updates) -> Sender<LocaleCommand> {
    let (sender, receiver) = mpsc::channel();
    let reader = system.clone();
    let feed = updates.clone();
    spawn("locale", move || {
        let follow = || -> zbus::Result<()> {
            let installed = installed_locales(&listing(), Path::new(LOCALE_SOURCES));
            let locale = LocaleProxyBlocking::builder(&reader)
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()?;
            let changes = PropertiesProxy::builder(&reader)
                .destination(LOCALE)?
                .path(LOCALE_PATH)?
                .build()?
                .receive_properties_changed()?;
            if !feed.send(Update::Locale(read(&locale, &installed))) {
                return Ok(());
            }
            for _ in changes {
                if !feed.send(Update::Locale(read(&locale, &installed))) {
                    break;
                }
            }
            Ok(())
        };
        if let Err(error) = follow() {
            tracing::warn!("cannot follow localed: {error}");
        }
    });
    let system = system.clone();
    spawn("locale-worker", move || {
        let Ok(locale) = LocaleProxyBlocking::builder(&system)
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
        else {
            return;
        };
        for command in receiver {
            let current = match locale.locale() {
                Ok(current) => current,
                Err(error) => {
                    updates.send(unexpected(error));
                    continue;
                }
            };
            let (language, region) = parse_assignments(&current);
            let (setting, mut wanted) = match command {
                LocaleCommand::Language(language) => (
                    SettingId::Language,
                    assignments(&language, region.as_deref()),
                ),
                LocaleCommand::Region(region) => {
                    (SettingId::Region, assignments(&language, region.as_deref()))
                }
            };
            wanted.extend(current.into_iter().filter(|assignment| {
                let name = assignment.split_once('=').map_or("", |(name, _)| name);
                name != "LANG" && name != "LANGUAGE" && !REGION_CATEGORIES.contains(&name)
            }));
            if let Err(error) = locale.set_locale(wanted, true) {
                updates.send(failed(setting, &error, "service-locale"));
                let installed = installed_locales(&listing(), Path::new(LOCALE_SOURCES));
                updates.send(Update::Locale(read(&locale, &installed)));
            }
        }
    });
    sender
}
