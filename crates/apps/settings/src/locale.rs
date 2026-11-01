use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, OnceLock};

use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;

use crate::failure::{failed, unexpected};
use crate::follow::{Update, Updates, spawn};
use bande_settings_panes::SettingId;

const LOCALE: &str = "org.freedesktop.locale1";
const LOCALE_PATH: &str = "/org/freedesktop/locale1";
const LOCALE_SOURCES: &str = "/usr/share/i18n/locales";
const SUPPORTED: &str = "/usr/share/i18n/SUPPORTED";
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
    #[zbus(property, name = "X11Layout")]
    fn x11_layout(&self) -> zbus::Result<String>;
    #[zbus(property, name = "X11Variant")]
    fn x11_variant(&self) -> zbus::Result<String>;
    #[zbus(name = "SetX11Keyboard")]
    fn set_x11_keyboard(
        &self,
        layout: &str,
        model: &str,
        variant: &str,
        options: &str,
        convert: bool,
        interactive: bool,
    ) -> zbus::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableLocale {
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocaleState {
    pub available: Vec<AvailableLocale>,
    pub language: String,
    pub region: Option<String>,
    pub pending: bool,
    pub login_sources: Vec<String>,
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
    LoginKeyboard(Vec<String>),
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

fn available_locale(entry: &str, sources: &Path) -> AvailableLocale {
    let (locale, modifier) = entry
        .split_once('@')
        .map_or((entry, None), |(locale, modifier)| (locale, Some(modifier)));
    let territory = base(locale);
    let source = modifier.map_or_else(
        || territory.to_owned(),
        |modifier| format!("{territory}@{modifier}"),
    );
    let name = locale_name(
        &source,
        &std::fs::read_to_string(sources.join(&source)).unwrap_or_default(),
    );
    AvailableLocale {
        code: modifier.map_or_else(
            || format!("{territory}.UTF-8"),
            |modifier| format!("{territory}.UTF-8@{modifier}"),
        ),
        name: match modifier {
            Some(modifier) if name != source => format!("{name}, {modifier}"),
            _ => name,
        },
    }
}

pub fn supported_locales(supported: &str, sources: &Path) -> Vec<AvailableLocale> {
    let mut available: Vec<AvailableLocale> = supported
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let entry = fields.next()?;
            (fields.next() == Some("UTF-8") && base(entry).contains('_')).then_some(entry)
        })
        .map(|entry| available_locale(entry, sources))
        .collect();
    available.sort_by_key(|locale| locale.name.to_lowercase());
    available.dedup_by(|a, b| a.code == b.code);
    available
}

fn read_supported() -> Vec<AvailableLocale> {
    supported_locales(
        &std::fs::read_to_string(SUPPORTED).unwrap_or_default(),
        Path::new(LOCALE_SOURCES),
    )
}

pub fn login_codes(layouts: &str, variants: &str) -> Vec<String> {
    let mut variants = variants.split(',').map(str::trim);
    layouts
        .split(',')
        .map(str::trim)
        .map(
            |layout| match variants.next().filter(|variant| !variant.is_empty()) {
                Some(variant) => format!("{layout}({variant})"),
                None => layout.to_owned(),
            },
        )
        .filter(|code| !code.is_empty())
        .collect()
}

pub fn x11_keyboard(codes: &[String]) -> (String, String) {
    let (layouts, variants): (Vec<&str>, Vec<&str>) = codes
        .iter()
        .map(|code| {
            code.strip_suffix(')')
                .and_then(|code| code.split_once('('))
                .unwrap_or((code, ""))
        })
        .unzip();
    (layouts.join(","), variants.join(","))
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

fn read(locale: &LocaleProxyBlocking<'_>, available: &[AvailableLocale]) -> Option<LocaleState> {
    let (language, region) = parse_assignments(&locale.locale().ok()?);
    Some(LocaleState {
        available: available.to_vec(),
        language,
        region,
        pending: false,
        login_sources: login_codes(
            &locale.x11_layout().unwrap_or_default(),
            &locale.x11_variant().unwrap_or_default(),
        ),
    })
}

pub fn follow_locale(system: &Connection, updates: Updates) -> Sender<LocaleCommand> {
    let (sender, receiver) = mpsc::channel();
    let reader = system.clone();
    let feed = updates.clone();
    let supported: Arc<OnceLock<Vec<AvailableLocale>>> = Arc::default();
    let cached = Arc::clone(&supported);
    spawn("locale", move || {
        let follow = || -> zbus::Result<()> {
            let available = cached.get_or_init(read_supported);
            let locale = LocaleProxyBlocking::builder(&reader)
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()?;
            let changes = PropertiesProxy::builder(&reader)
                .destination(LOCALE)?
                .path(LOCALE_PATH)?
                .build()?
                .receive_properties_changed()?;
            if !feed.send(Update::Locale(read(&locale, available))) {
                return Ok(());
            }
            for _ in changes {
                if !feed.send(Update::Locale(read(&locale, available))) {
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
            if let LocaleCommand::LoginKeyboard(codes) = &command {
                let (layouts, variants) = x11_keyboard(codes);
                if let Err(error) =
                    locale.set_x11_keyboard(&layouts, "", &variants, "", false, true)
                {
                    updates.send(failed(SettingId::InputSources, &error, "service-locale"));
                    updates.send(Update::Locale(read(
                        &locale,
                        supported.get_or_init(read_supported),
                    )));
                }
                continue;
            }
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
                LocaleCommand::LoginKeyboard(_) => continue,
            };
            wanted.extend(current.into_iter().filter(|assignment| {
                let name = assignment.split_once('=').map_or("", |(name, _)| name);
                name != "LANG" && name != "LANGUAGE" && !REGION_CATEGORIES.contains(&name)
            }));
            if let Err(error) = locale.set_locale(wanted, true) {
                updates.send(failed(setting, &error, "service-locale"));
                updates.send(Update::Locale(read(
                    &locale,
                    supported.get_or_init(read_supported),
                )));
            }
        }
    });
    sender
}
