use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use spellbook::Dictionary;
use zbus::object_server::SignalEmitter;

use crate::dictionaries::{
    bande_data_home, best_for, data_directories, input_source_language, installed, language_data,
    language_of, load,
};
use crate::store::Store;

pub const BUS_NAME: &str = "org.bande.Spell";
pub const OBJECT_PATH: &str = "/org/bande/Spell";
const IDLE_EXIT: Duration = Duration::from_secs(60);

pub struct Engine {
    store: Store,
    region: Option<String>,
    system_language: Option<String>,
    input_languages: Vec<String>,
    loaded: HashMap<String, Dictionary>,
    variants: HashMap<String, Dictionary>,
    preferred: Vec<String>,
    common: HashMap<String, HashSet<String>>,
}

impl Engine {
    fn active(&self) -> Vec<String> {
        let everything = installed();
        let wanted: Vec<String> = if self.store.languages.is_empty() {
            self.system_language
                .iter()
                .cloned()
                .chain(["en".to_owned()])
                .chain(self.input_languages.iter().cloned())
                .collect()
        } else {
            self.store.languages.clone()
        };
        let mut active = Vec::new();
        for language in wanted {
            if let Some(name) = best_for(&language, self.region_for(&language), &everything)
                && !active.contains(&name)
            {
                active.push(name);
            }
        }
        active
    }

    fn region_for(&self, language: &str) -> Option<&str> {
        self.preferred
            .iter()
            .filter(|code| language_of(code) == language_of(language))
            .find_map(|code| code.split_once('_').map(|(_, region)| region))
            .or(self.region.as_deref())
    }

    pub fn missing(&self) -> Vec<String> {
        let everything = installed();
        let mut missing = Vec::new();
        for name in self.active() {
            let language = language_of(&name).to_owned();
            if let Some(region) = self.region_for(&language) {
                let wanted = format!("{language}_{region}");
                if !everything.contains_key(&wanted) && !missing.contains(&wanted) {
                    missing.push(wanted);
                }
            }
        }
        missing
    }

    fn valid_in_a_variant(&mut self, word: &str) -> bool {
        let active = self.active();
        let languages: Vec<&str> = active.iter().map(|name| language_of(name)).collect();
        for (name, path) in installed() {
            if active.contains(&name) || !languages.contains(&language_of(&name)) {
                continue;
            }
            if !self.variants.contains_key(&name)
                && let Some(dictionary) = load(&path)
            {
                self.variants.insert(name.clone(), dictionary);
            }
            if self
                .variants
                .get(&name)
                .is_some_and(|dictionary| dictionary.check(word))
            {
                return true;
            }
        }
        false
    }

    fn dictionaries(&mut self) -> Vec<&Dictionary> {
        let active = self.active();
        let everything = installed();
        self.loaded.retain(|name, _| active.contains(name));
        for name in &active {
            if !self.loaded.contains_key(name)
                && let Some(dictionary) = everything.get(name).and_then(|path| load(path))
            {
                self.loaded.insert(name.clone(), dictionary);
            }
        }
        active
            .iter()
            .filter_map(|name| self.loaded.get(name))
            .collect()
    }

    fn is_learned(&self, word: &str) -> bool {
        self.store.learned.contains(word) || self.store.learned.contains(&word.to_lowercase())
    }

    pub fn check(&mut self, words: &[String]) -> Vec<bool> {
        if self.store.options.get("spelling") == Some(&false) {
            return vec![true; words.len()];
        }
        let learned: Vec<bool> = words.iter().map(|word| self.is_learned(word)).collect();
        let dictionaries = self.dictionaries();
        if dictionaries.is_empty() {
            return vec![true; words.len()];
        }
        words
            .iter()
            .zip(learned)
            .map(|(word, learned)| {
                learned || dictionaries.iter().any(|dictionary| dictionary.check(word))
            })
            .collect()
    }

    fn common_words(&mut self, language: &str) -> &HashSet<String> {
        self.common.entry(language.to_owned()).or_insert_with(|| {
            data_directories()
                .iter()
                .find_map(|directory| {
                    fs::read_to_string(directory.join(format!("bande/text/{language}.common"))).ok()
                })
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        })
    }

    pub fn correction(&mut self, word: &str, sentence_start: bool) -> String {
        let capitalised = word.chars().next().is_some_and(char::is_uppercase);
        if self.store.options.get("autocorrect") == Some(&false)
            || !correctable(word)
            || (capitalised && !sentence_start)
            || self.check(&[word.to_owned()]) == [true]
            || self.valid_in_a_variant(word)
        {
            return String::new();
        }
        let typed = word.to_lowercase();
        self.dictionaries();
        let mut candidates: Vec<(String, bool)> = Vec::new();
        for name in self.active() {
            let language = language_of(&name).to_owned();
            let rules = language_data(&language);
            let long_enough = rules
                .get("autocorrect-min-length")
                .and_then(|value| value.parse::<usize>().ok())
                .is_some_and(|least| typed.chars().count() >= least);
            let enabled = rules
                .get("autocorrect")
                .is_some_and(|value| value == "true");
            if self.common_words(&language).contains(&typed) {
                return String::new();
            }
            let common = self.common_words(&language).clone();
            let Some(dictionary) = self.loaded.get(&name) else {
                continue;
            };
            let mut suggestions = Vec::new();
            dictionary.suggest(word, &mut suggestions);
            for suggestion in suggestions {
                if one_edit_apart(word, &suggestion) {
                    let strong = common.contains(&suggestion.to_lowercase());
                    candidates.push((suggestion, enabled && long_enough && strong));
                }
            }
        }
        let first = candidates.first().map(|(word, _)| word.clone());
        match first {
            Some(only)
                if candidates
                    .iter()
                    .all(|(candidate, allowed)| *candidate == only && *allowed) =>
            {
                only
            }
            _ => String::new(),
        }
    }

    pub fn suggest(&mut self, word: &str) -> Vec<String> {
        let mut merged: Vec<String> = Vec::new();
        for dictionary in self.dictionaries() {
            let mut suggestions = Vec::new();
            dictionary.suggest(word, &mut suggestions);
            for suggestion in suggestions {
                if !merged.contains(&suggestion) {
                    merged.push(suggestion);
                }
            }
        }
        merged
    }
}

fn correctable(word: &str) -> bool {
    let mut characters = word.chars();
    characters.next().is_some_and(char::is_alphabetic)
        && characters.all(|character| character.is_alphabetic() && !character.is_uppercase())
}

fn one_edit_apart(typed: &str, candidate: &str) -> bool {
    let typed: Vec<char> = typed.chars().flat_map(char::to_lowercase).collect();
    let candidate: Vec<char> = candidate.chars().flat_map(char::to_lowercase).collect();
    if typed == candidate || typed.len().abs_diff(candidate.len()) > 1 {
        return false;
    }
    let prefix = typed
        .iter()
        .zip(&candidate)
        .take_while(|(left, right)| left == right)
        .count();
    let (rest_typed, rest_candidate) = (&typed[prefix..], &candidate[prefix..]);
    let same_after = |skip_typed: usize, skip_candidate: usize| {
        rest_typed.get(skip_typed..) == rest_candidate.get(skip_candidate..)
    };
    same_after(1, 1)
        || same_after(1, 0)
        || same_after(0, 1)
        || (rest_typed.len() >= 2
            && rest_candidate.len() >= 2
            && rest_typed[0] == rest_candidate[1]
            && rest_typed[1] == rest_candidate[0]
            && same_after(2, 2))
}

pub struct Spell {
    engine: Arc<Mutex<Engine>>,
    activity: Sender<()>,
}

impl Spell {
    fn engine(&self) -> MutexGuard<'_, Engine> {
        let _ = self.activity.send(());
        self.engine.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn saved(result: std::io::Result<()>) -> zbus::fdo::Result<()> {
    result.map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
}

#[zbus::interface(name = "org.bande.Spell")]
impl Spell {
    fn check(&self, words: Vec<String>) -> Vec<bool> {
        self.engine().check(&words)
    }

    fn suggest(&self, word: String) -> Vec<String> {
        self.engine().suggest(&word)
    }

    fn rules(&self) -> (HashMap<String, String>, HashMap<String, String>) {
        let engine = self.engine();
        let mut rules = language_data("en");
        if let Some(language) = &engine.system_language {
            rules.extend(language_data(language));
        }
        for name in ["capitalise", "double-space", "smart-quotes", "links"] {
            let on = engine.store.options.get(name).copied().unwrap_or(true);
            rules.insert(format!("{name}-on"), on.to_string());
        }
        (rules, engine.store.replacements())
    }

    async fn set_replacements(
        &self,
        replacements: HashMap<String, String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        saved(self.engine().store.save_replacements(&replacements))?;
        Self::changed(&emitter).await?;
        Ok(())
    }

    fn correction(&self, word: String, sentence_start: bool) -> String {
        self.engine().correction(&word, sentence_start)
    }

    async fn learn(
        &self,
        word: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        {
            let mut engine = self.engine();
            engine.store.learned.insert(word);
            saved(engine.store.save_learned())?;
        }
        Self::changed(&emitter).await?;
        Ok(())
    }

    async fn forget(
        &self,
        word: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        {
            let mut engine = self.engine();
            engine.store.learned.remove(&word);
            saved(engine.store.save_learned())?;
        }
        Self::changed(&emitter).await?;
        Ok(())
    }

    fn learned_words(&self) -> Vec<String> {
        self.engine().store.learned.iter().cloned().collect()
    }

    fn options(&self) -> BTreeMap<String, bool> {
        self.engine().store.options.clone()
    }

    async fn set_option(
        &self,
        name: String,
        value: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        {
            let mut engine = self.engine();
            engine.store.options.insert(name, value);
            saved(engine.store.save_settings())?;
        }
        Self::changed(&emitter).await?;
        Ok(())
    }

    fn languages(&self) -> (Vec<String>, Vec<String>, Vec<String>) {
        let engine = self.engine();
        (
            engine.store.languages.clone(),
            engine.active(),
            installed().into_keys().collect(),
        )
    }

    fn missing_dictionaries(&self) -> Vec<String> {
        self.engine().missing()
    }

    async fn set_languages(
        &self,
        languages: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        {
            let mut engine = self.engine();
            engine.store.languages = languages;
            saved(engine.store.save_settings())?;
        }
        Self::changed(&emitter).await?;
        Ok(())
    }

    #[zbus(signal)]
    async fn changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

fn system_locale() -> (Option<String>, Option<String>) {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()));
    let Some(locale) = locale.filter(|locale| locale != "C" && locale != "POSIX") else {
        return (None, None);
    };
    let code = bande_l10n::system::base(&locale);
    let mut parts = code.split('_');
    let language = parts.next().map(str::to_owned);
    (language, parts.next().map(str::to_owned))
}

fn input_languages(config: &std::path::Path) -> Vec<String> {
    let source = std::fs::read_to_string(config.join("bande/config.kdl")).unwrap_or_default();
    let sources = bande_config::Config::load(&source)
        .map(|(config, _)| config.keyboard.input_sources)
        .unwrap_or_default();
    let sources = if sources.is_empty() {
        bande_config::read_system_input_sources()
    } else {
        sources
    };
    let table = language_data("input-sources");
    sources
        .iter()
        .map(|source| input_source_language(&source.layout, &table))
        .collect()
}

fn wait_until_idle(activity: &Receiver<()>) {
    loop {
        match activity.recv_timeout(IDLE_EXIT) {
            Ok(()) => {}
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return,
        }
    }
}

impl Engine {
    pub fn open() -> Result<Self, Box<dyn std::error::Error>> {
        let home = std::env::home_dir();
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(".config")))
            .ok_or("neither XDG_CONFIG_HOME nor HOME is set")?;
        let data = bande_data_home().ok_or("neither XDG_DATA_HOME nor HOME is set")?;
        let (system_language, region) = system_locale();
        Ok(Self {
            store: Store::open(&config, &data),
            region,
            system_language,
            input_languages: input_languages(&config),
            loaded: HashMap::new(),
            variants: HashMap::new(),
            preferred: std::env::var("LANGUAGE")
                .unwrap_or_default()
                .split(':')
                .filter(|code| code.contains('_'))
                .map(|code| code.split(['.', '@']).next().unwrap_or(code).to_owned())
                .collect(),
            common: HashMap::new(),
        })
    }
}

pub fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Engine::open()?;
    let (activity, idle) = channel();
    let spell = Spell {
        engine: Arc::new(Mutex::new(engine)),
        activity,
    };
    let connection = bande_bus::session()?;
    connection.object_server().at(OBJECT_PATH, spell)?;
    connection.request_name(BUS_NAME)?;
    wait_until_idle(&idle);
    connection.release_name(BUS_NAME)?;
    Ok(())
}
