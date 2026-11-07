use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, OnceLock};

use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;

use crate::L10N;
use crate::failure::{detail, failed, sentence_or};
use crate::follow::{Update, Updates, spawn};
use bande_settings_panes::SettingId;

const LOCALE: &str = "org.freedesktop.locale1";
const LOCALE_PATH: &str = "/org/freedesktop/locale1";
const LOCALE_SOURCES: &str = "/usr/share/i18n/locales";
const SUPPORTED: &str = "/usr/share/i18n/SUPPORTED";
const REFUSED_LOCALE: &str = "InvalidArgs";
const DEFAULT_LOCALE: &str = "C.UTF-8";
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
    pub pending: Option<SettingId>,
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

pub(crate) fn same_locale(a: &str, b: &str) -> bool {
    let key = |code: &str| {
        let (name, modifier) = code.split_once('@').unwrap_or((code, ""));
        format!("{}@{modifier}", base(name)).to_lowercase()
    };
    key(a) == key(b)
}

pub(crate) fn locale_label(available: &[AvailableLocale], code: &str) -> String {
    if same_locale(code, DEFAULT_LOCALE) {
        return L10N.tr("locale-default");
    }
    available
        .iter()
        .find(|locale| same_locale(&locale.code, code))
        .map_or_else(|| code.to_owned(), |locale| locale.name.clone())
}

fn available_locale(entry: &str, sources: &Path) -> AvailableLocale {
    let modifier = entry.split_once('@').map(|(_, modifier)| modifier);
    let territory = base(entry);
    let source = modifier.map_or_else(
        || territory.to_owned(),
        |modifier| format!("{territory}@{modifier}"),
    );
    let name = locale_name(
        &source,
        &std::fs::read_to_string(sources.join(&source)).unwrap_or_default(),
    );
    AvailableLocale {
        code: entry.to_owned(),
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

fn assigned<'a>(assignments: &'a [String], name: &str) -> Option<&'a str> {
    assignments
        .iter()
        .find_map(|assignment| assignment.strip_prefix(name)?.strip_prefix('='))
        .filter(|value| !value.is_empty())
}

pub fn parse_assignments(assignments: &[String]) -> (String, Option<String>) {
    let language = assigned(assignments, "LANG").unwrap_or(DEFAULT_LOCALE);
    let region = assigned(assignments, REGION_CATEGORIES[0])
        .filter(|region| !same_locale(region, language))
        .map(str::to_owned);
    (language.to_owned(), region)
}

pub fn assignments(current: &[String], language: &str, region: Option<&str>) -> Vec<String> {
    let mut wanted = vec![format!("LANG={language}")];
    let (current_language, _) = parse_assignments(current);
    if assigned(current, "LANGUAGE").is_some() && !same_locale(&current_language, language) {
        wanted.push(format!("LANGUAGE={}", language.replacen(".UTF-8", "", 1)));
    }
    let region = region.filter(|region| !same_locale(region, language));
    if region.is_some()
        || REGION_CATEGORIES
            .iter()
            .any(|category| assigned(current, category).is_some())
    {
        let formats = region.unwrap_or(language);
        wanted.extend(
            REGION_CATEGORIES
                .iter()
                .map(|category| format!("{category}={formats}")),
        );
    }
    wanted
}

fn change_failure(error: &zbus::Error, code: &str, available: &[AvailableLocale]) -> String {
    let name = locale_label(available, code);
    sentence_or(error, "service-locale", |kind| {
        if kind == REFUSED_LOCALE {
            L10N.tr_args("locale-unavailable", [("name", name.clone().into())])
        } else {
            L10N.tr_args(
                "locale-not-set-up",
                [
                    ("name", name.clone().into()),
                    (
                        "reason",
                        detail(error)
                            .unwrap_or_else(|| L10N.tr("couldnt-change"))
                            .into(),
                    ),
                ],
            )
        }
    })
}

fn read(locale: &LocaleProxyBlocking<'_>, available: &[AvailableLocale]) -> Option<LocaleState> {
    let (language, region) = parse_assignments(&locale.locale().ok()?);
    Some(LocaleState {
        available: available.to_vec(),
        language,
        region,
        pending: None,
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
            let available = supported.get_or_init(read_supported);
            let (setting, chosen) = match command {
                LocaleCommand::Language(language) => (SettingId::Language, Some(language)),
                LocaleCommand::Region(region) => (SettingId::Region, region),
                LocaleCommand::LoginKeyboard(_) => continue,
            };
            let current = match locale.locale() {
                Ok(current) => current,
                Err(error) => {
                    updates.send(failed(setting, &error, "service-locale"));
                    continue;
                }
            };
            let (language, region) = parse_assignments(&current);
            let wanted = if setting == SettingId::Language {
                assignments(
                    &current,
                    chosen.as_deref().unwrap_or(&language),
                    region.as_deref(),
                )
            } else {
                assignments(&current, &language, chosen.as_deref())
            };
            if let Err(error) = locale.set_locale(wanted, true) {
                let code = chosen.as_deref().unwrap_or(&language);
                updates.send(Update::Failed(
                    Some(setting),
                    change_failure(&error, code, available),
                ));
            }
            updates.send(Update::Locale(read(&locale, available)));
        }
    });
    sender
}
