use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use spellbook::Dictionary;
use zbus::object_server::SignalEmitter;

use crate::dictionaries::{
    bande_data_home, best_for, input_source_language, installed, language_data, language_of, load,
};
use crate::store::Store;

pub const BUS_NAME: &str = "org.bande.Spell";
pub const OBJECT_PATH: &str = "/org/bande/Spell";
const IDLE_EXIT: Duration = Duration::from_secs(60);

struct Engine {
    store: Store,
    region: Option<String>,
    system_language: Option<String>,
    input_languages: Vec<String>,
    loaded: HashMap<String, Dictionary>,
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
            let region = self
                .region
                .as_deref()
                .filter(|_| self.system_language.as_deref() == Some(language_of(&language)));
            if let Some(name) = best_for(&language, region, &everything)
                && !active.contains(&name)
            {
                active.push(name);
            }
        }
        active
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

    fn check(&mut self, words: &[String]) -> Vec<bool> {
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

    fn suggest(&mut self, word: &str) -> Vec<String> {
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

pub fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::home_dir();
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".config")))
        .ok_or("neither XDG_CONFIG_HOME nor HOME is set")?;
    let data = bande_data_home().ok_or("neither XDG_DATA_HOME nor HOME is set")?;
    let (system_language, region) = system_locale();
    let engine = Engine {
        store: Store::open(&config, &data),
        region,
        system_language,
        input_languages: input_languages(&config),
        loaded: HashMap::new(),
    };
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
