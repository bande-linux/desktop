use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use bande_l10n::{text, tr};

use crate::failure::explained;
use crate::follow::{Update, Updates, spawn};
use bande_settings_panes::{PaneId, SettingId};
use zbus::blocking::Connection;

pub use bande_l10n::system::{AvailableLocale, base, read_supported, supported_locales};

const HELPER: &str = "/usr/libexec/bande-locale-helper";
const LANGUAGE_PACKS: &str = "bande-language-packs.service";
const PACKS: &str = "bande-desktop-l10n-";
const OWN_CATALOGS: &str = "bande-settings";
const PKEXEC: &str = "/usr/bin/pkexec";
const HELPER_REFUSED_VALUE: i32 = 2;
const PKEXEC_DISMISSED: i32 = 126;
const PKEXEC_NOT_AUTHORIZED: i32 = 127;
pub(crate) const DEFAULT_LOCALE: &str = "C.UTF-8";
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
    #[zbus(property, name = "X11Layout")]
    fn x11_layout(&self) -> zbus::Result<String>;
    #[zbus(property, name = "X11Variant")]
    fn x11_variant(&self) -> zbus::Result<String>;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocaleState {
    pub available: Vec<AvailableLocale>,
    pub language: String,
    pub region: Option<String>,
    pub pending: Option<SettingId>,
    pub login_sources: Vec<String>,
    pub translations: Vec<String>,
    pub packs: Packs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Packs {
    #[default]
    Ready,
    Fetching,
    Offline,
}

#[derive(Default)]
struct PackWatch {
    generation: u64,
    packs: Packs,
}

type SharedPacks = Arc<Mutex<PackWatch>>;

impl PackWatch {
    fn begin(&mut self) -> u64 {
        self.generation += 1;
        self.packs = Packs::Fetching;
        self.generation
    }

    fn finish(&mut self, generation: u64, packs: Packs) -> bool {
        let current = generation == self.generation;
        if current {
            self.packs = packs;
        }
        current
    }
}

fn packs_now(watch: &SharedPacks) -> Packs {
    watch.lock().unwrap_or_else(PoisonError::into_inner).packs
}

fn pack_outcome(fetched: bool, installed: bool, offered: bool) -> Packs {
    if fetched && installed || !offered {
        Packs::Ready
    } else {
        Packs::Offline
    }
}

impl LocaleState {
    pub fn translated(&self, code: &str) -> bool {
        let tongue = tongue(code);
        matches!(tongue.as_str(), "c" | "posix") || self.translations.contains(&tongue)
    }

    pub fn territory(&self) -> Option<&str> {
        let code = base(self.region.as_deref().unwrap_or(&self.language));
        code.split_once('_').map(|(_, territory)| territory)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocaleCommand {
    Read,
    Language(String),
    Region(Option<String>),
    LoginKeyboard(Vec<String>),
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
        return tr!("English (no country)");
    }
    available
        .iter()
        .find(|locale| same_locale(&locale.code, code))
        .map_or_else(|| code.to_owned(), |locale| locale.name.clone())
}

fn login_codes(layouts: &str, variants: &str) -> Vec<String> {
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

fn assigned<'a>(assignments: &'a [String], name: &str) -> Option<&'a str> {
    assignments
        .iter()
        .find_map(|assignment| assignment.strip_prefix(name)?.strip_prefix('='))
        .filter(|value| !value.is_empty())
}

fn parse_assignments(assignments: &[String]) -> (String, Option<String>) {
    let language = assigned(assignments, "LANG").unwrap_or(DEFAULT_LOCALE);
    let region = assigned(assignments, REGION_CATEGORIES[0])
        .filter(|region| !same_locale(region, language))
        .map(str::to_owned);
    (language.to_owned(), region)
}

fn assignments(current: &[String], language: &str, region: Option<&str>) -> Vec<String> {
    let mut wanted = vec![format!("LANG={language}")];
    let (current_language, _) = parse_assignments(current);
    if assigned(current, "LANGUAGE").is_some() && !same_locale(&current_language, language) {
        wanted.push(format!("LANGUAGE={}", language.replacen(".UTF-8", "", 1)));
    }
    let formats = region
        .filter(|region| !same_locale(region, language))
        .unwrap_or_default();
    wanted.extend(
        REGION_CATEGORIES
            .iter()
            .map(|category| format!("{category}={formats}")),
    );
    wanted
}

enum HelperFailure {
    NotAllowed,
    UnknownValue,
    Failed(String),
}

fn run_helper(arguments: &[String]) -> Result<(), HelperFailure> {
    let output = Command::new(PKEXEC)
        .arg(HELPER)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| HelperFailure::Failed(error.to_string()))?;
    match output.status.code() {
        Some(0) => Ok(()),
        Some(PKEXEC_DISMISSED | PKEXEC_NOT_AUTHORIZED) => Err(HelperFailure::NotAllowed),
        Some(HELPER_REFUSED_VALUE) => Err(HelperFailure::UnknownValue),
        _ => Err(HelperFailure::Failed(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        )),
    }
}

fn change_failure(failure: HelperFailure, code: &str, available: &[AvailableLocale]) -> String {
    let name = locale_label(available, code);
    match failure {
        HelperFailure::NotAllowed => tr!("Only an administrator can change this."),
        HelperFailure::UnknownValue => tr!("{name} can't be used on this computer.", name = name),
        HelperFailure::Failed(reason) => {
            tracing::warn!("locale helper: {reason}");
            tr!(
                "Couldn't set up {name}. {reason}",
                name = name,
                reason = reason
            )
        }
    }
}

fn keyboard_failure(failure: HelperFailure) -> String {
    match failure {
        HelperFailure::NotAllowed => tr!("Only an administrator can change this."),
        HelperFailure::UnknownValue => tr!("Couldn't make this change."),
        HelperFailure::Failed(reason) => {
            tracing::warn!("locale helper: {reason}");
            tr!("Couldn't make this change.")
        }
    }
}

fn user_file() -> Option<PathBuf> {
    Some(bande_xdg::config_home()?.join("locale.conf"))
}

fn with_user_choices(system: Vec<String>) -> Vec<String> {
    let user = user_file().and_then(|path| std::fs::read_to_string(path).ok());
    overlaid(system, user.as_deref().unwrap_or_default())
}

fn overlaid(system: Vec<String>, user: &str) -> Vec<String> {
    let chosen: Vec<&str> = user.lines().filter(|line| line.contains('=')).collect();
    let key = |line: &str| line.split('=').next().unwrap_or_default().to_owned();
    let mut merged: Vec<String> = system
        .into_iter()
        .filter(|line| !chosen.iter().any(|mine| key(mine) == key(line)))
        .collect();
    merged.extend(chosen.iter().map(|line| (*line).to_owned()));
    merged
}

fn user_lines(wanted: &[String], language: &str) -> String {
    wanted
        .iter()
        .map(|line| match line.strip_suffix('=') {
            Some(key) => format!("{key}={language}\n"),
            None => format!("{line}\n"),
        })
        .collect()
}

fn write_user_file(contents: &str) -> std::io::Result<()> {
    let path = user_file().ok_or(std::io::ErrorKind::NotFound)?;
    bande_xdg::replace_file(&path, contents)
}

fn generated(code: &str) -> bool {
    let normal = |name: &str| name.to_lowercase().replace("utf-8", "utf8");
    Command::new("/usr/bin/locale")
        .arg("-a")
        .output()
        .is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|name| normal(name) == normal(code))
        })
}

fn read(
    locale: &LocaleProxyBlocking<'_>,
    available: &[AvailableLocale],
    watch: &SharedPacks,
) -> Option<LocaleState> {
    let (language, region) = parse_assignments(&with_user_choices(locale.locale().ok()?));
    Some(LocaleState {
        available: available.to_vec(),
        language,
        region,
        pending: None,
        translations: translations(),
        packs: packs_now(watch),
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
    let watch = SharedPacks::default();
    let cached = Arc::clone(&supported);
    let shown_packs = Arc::clone(&watch);
    spawn("locale", move || {
        let available = cached.get_or_init(read_supported);
        match locale_proxy(&reader) {
            Ok(locale) => {
                feed.send(Update::Locale(read(&locale, available, &shown_packs)));
            }
            Err(error) => tracing::warn!("cannot read localed: {error}"),
        }
    });
    let system = system.clone();
    spawn("locale-worker", move || {
        let Ok(locale) = locale_proxy(&system) else {
            return;
        };
        for command in receiver {
            let available = supported.get_or_init(read_supported);
            if command == LocaleCommand::Read {
                updates.send(Update::Locale(read(&locale, available, &watch)));
                continue;
            }
            if let LocaleCommand::LoginKeyboard(codes) = &command {
                let mut arguments = vec!["keyboard".to_owned()];
                arguments.extend_from_slice(codes);
                if let Err(failure) = run_helper(&arguments) {
                    updates.send(Update::Failed(
                        SettingId::InputSources,
                        keyboard_failure(failure),
                    ));
                }
                updates.send(Update::Locale(read(&locale, available, &watch)));
                continue;
            }
            let (setting, chosen) = match command {
                LocaleCommand::Language(language) => (SettingId::Language, Some(language)),
                LocaleCommand::Region(region) => (SettingId::Region, region),
                LocaleCommand::Read | LocaleCommand::LoginKeyboard(_) => continue,
            };
            let current = match locale.locale() {
                Ok(current) => with_user_choices(current),
                Err(error) => {
                    updates.send(Update::PaneFailed(
                        PaneId::LanguageTime,
                        explained(
                            &error,
                            text!("The language service"),
                            text!("Couldn't read the current language and region."),
                        ),
                    ));
                    updates.send(Update::Locale(read(&locale, available, &watch)));
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
            let shown = if setting == SettingId::Language {
                chosen.clone().unwrap_or_else(|| language.clone())
            } else {
                language.clone()
            };
            let needed = chosen.as_deref().unwrap_or(&language);
            let contents = user_lines(&wanted, &shown);
            let mut arguments = vec!["locale".to_owned()];
            arguments.extend(wanted);
            let outcome = match run_helper(&arguments) {
                Err(HelperFailure::NotAllowed) if generated(needed) => Ok(()),
                other => other,
            };
            match outcome {
                Ok(()) => match write_user_file(&contents) {
                    Ok(()) if setting == SettingId::Language => {
                        follow_packs(&system, &updates, &watch, available, tongue(needed));
                    }
                    Ok(()) => {}
                    Err(error) => {
                        tracing::warn!("cannot write the language choice: {error}");
                        updates.send(Update::Failed(setting, tr!("Couldn't make this change.")));
                    }
                },
                Err(failure) => {
                    updates.send(Update::Failed(
                        setting,
                        change_failure(failure, needed, available),
                    ));
                }
            }
            updates.send(Update::Locale(read(&locale, available, &watch)));
        }
    });
    sender
}

fn locale_proxy(system: &Connection) -> zbus::Result<LocaleProxyBlocking<'static>> {
    LocaleProxyBlocking::builder(system)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}

fn follow_packs(
    system: &Connection,
    updates: &Updates,
    watch: &SharedPacks,
    available: &[AvailableLocale],
    wanted: String,
) {
    let generation = watch.lock().unwrap_or_else(PoisonError::into_inner).begin();
    let system = system.clone();
    let updates = updates.clone();
    let watch = Arc::clone(watch);
    let available = available.to_vec();
    fetch_language_packs(move |fetched| {
        let installed = installed_catalogs().contains(&wanted);
        let offered = installed || translations().contains(&wanted);
        let current = watch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .finish(generation, pack_outcome(fetched, installed, offered));
        if current && let Ok(locale) = locale_proxy(&system) {
            updates.send(Update::Locale(read(&locale, &available, &watch)));
        }
    });
}

pub(crate) fn fetch_language_packs(done: impl FnOnce(bool) + Send + 'static) {
    spawn("language-packs", move || {
        let fetched = bande_bus::system()
            .map_err(Into::into)
            .and_then(|system| crate::software::versions::run_unit(&system, LANGUAGE_PACKS));
        done(
            fetched
                .inspect_err(|error| tracing::warn!("cannot fetch the language packs: {error}"))
                .is_ok(),
        );
    });
}

fn tongue(code: &str) -> String {
    code.split(['_', '-', '.', '@'])
        .next()
        .unwrap_or(code)
        .to_lowercase()
}

fn installed_catalogs() -> Vec<String> {
    std::fs::read_dir(std::path::Path::new(bande_l10n::CATALOGS).join(OWN_CATALOGS))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            name.strip_suffix(".mo").map(tongue)
        })
        .collect()
}

fn translations() -> Vec<String> {
    let mut found = installed_catalogs();
    match Command::new("apt-cache").args(["pkgnames", PACKS]).output() {
        Ok(output) => found.extend(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|name| name.strip_prefix(PACKS))
                .map(tongue),
        ),
        Err(error) => tracing::warn!("cannot list the language packs: {error}"),
    }
    found.sort();
    found.dedup();
    found
}

#[cfg(test)]
mod per_user_tests {
    use super::*;

    #[test]
    fn a_later_language_choice_owns_the_pack_row() {
        let mut watch = PackWatch::default();
        let first = watch.begin();
        let second = watch.begin();
        assert!(!watch.finish(first, pack_outcome(true, true, true)));
        assert_eq!(watch.packs, Packs::Fetching);
        assert!(watch.finish(second, pack_outcome(false, false, true)));
        assert_eq!(watch.packs, Packs::Offline);
        assert_eq!(pack_outcome(false, false, false), Packs::Ready);
    }

    #[test]
    fn the_users_choice_wins_and_clearing_formats_overrides_the_system() {
        let merged = overlaid(
            vec![
                "LANG=en_US.UTF-8".to_owned(),
                "LC_TIME=de_DE.UTF-8".to_owned(),
            ],
            &user_lines(
                &["LANG=sv_SE.UTF-8".to_owned(), "LC_TIME=".to_owned()],
                "sv_SE.UTF-8",
            ),
        );
        let (language, region) = parse_assignments(&merged);
        assert_eq!(language, "sv_SE.UTF-8");
        assert_eq!(region, None);
    }
}
