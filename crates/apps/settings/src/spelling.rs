use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};

use bande_toolkit::ui::{Button, TextField, Toggle};
use zbus::blocking::{Connection, Proxy};

use crate::L10N;
use crate::follow::{Command, Update, Updates, spawn};
use crate::pages::{Widgets, boxed, muted_row};
use crate::view::{Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

const BUS_NAME: &str = "org.bande.Spell";
const OBJECT_PATH: &str = "/org/bande/Spell";
const SWITCHES: [(&str, &str); 6] = [
    ("spelling", "spelling-check"),
    ("autocorrect", "spelling-autocorrect"),
    ("capitalise", "spelling-capitalise"),
    ("double-space", "spelling-double-space"),
    ("smart-quotes", "spelling-smart-quotes"),
    ("links", "spelling-links"),
];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpellingState {
    options: BTreeMap<String, bool>,
    chosen: Vec<String>,
    active: Vec<String>,
    available: Vec<String>,
    learned: Vec<String>,
    missing: Vec<String>,
    replacements: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SpellingCommand {
    Read,
    Switch(String, bool),
    Dictionary(String, bool),
    Forget(String),
    GetDictionary(String),
    SetReplacement(String, Option<String>),
}

fn read(proxy: &Proxy<'_>) -> zbus::Result<SpellingState> {
    let (chosen, active, available) = proxy.call("Languages", &())?;
    let (_, replacements): (BTreeMap<String, String>, BTreeMap<String, String>) =
        proxy.call("Rules", &())?;
    Ok(SpellingState {
        options: proxy.call("Options", &())?,
        chosen,
        active,
        available,
        learned: proxy.call("LearnedWords", &())?,
        missing: proxy
            .call::<_, _, Vec<String>>("MissingDictionaries", &())?
            .into_iter()
            .filter(|name| dictionary_package(name).is_some())
            .collect(),
        replacements,
    })
}

fn package_names(locale: &str) -> Vec<String> {
    let code = locale.split(['.', '@']).next().unwrap_or(locale);
    let mut parts = code.split('_');
    let language = parts.next().unwrap_or(code).to_lowercase();
    let mut names = vec![format!("hunspell-{language}")];
    if let Some(region) = parts.next() {
        names.insert(0, format!("hunspell-{language}-{}", region.to_lowercase()));
    }
    names
}

fn dictionary_package(name: &str) -> Option<String> {
    let system = bande_bus::system().ok()?;
    let packages = bande_packages::Client::new(&system);
    let names = package_names(name);
    let exact = names.first()?;
    packages
        .resolve(std::slice::from_ref(exact))
        .ok()?
        .into_iter()
        .next()
        .map(|package| package.id)
}

fn get_dictionary(locale: &str) -> Result<(), Box<dyn std::error::Error>> {
    let names = package_names(locale);
    let system = bande_bus::system()?;
    let packages = bande_packages::Client::new(&system);
    let found = packages.resolve(&names)?;
    let Some(package) = found.first() else {
        return Ok(());
    };
    packages.install(
        std::slice::from_ref(&package.id),
        bande_packages::Mode::Apply,
        &|_| {},
    )?;
    Ok(())
}

fn apply(proxy: &Proxy<'_>, command: SpellingCommand, state: &SpellingState) -> zbus::Result<()> {
    match command {
        SpellingCommand::Read => Ok(()),
        SpellingCommand::Switch(name, on) => proxy.call("SetOption", &(name, on)),
        SpellingCommand::Forget(word) => proxy.call("Forget", &(word,)),
        SpellingCommand::Dictionary(name, on) => {
            let mut chosen = if state.chosen.is_empty() {
                state.active.clone()
            } else {
                state.chosen.clone()
            };
            chosen.retain(|dictionary| *dictionary != name);
            if on {
                chosen.push(name);
            }
            proxy.call("SetLanguages", &(chosen,))
        }
        SpellingCommand::SetReplacement(from, to) => {
            let mut all = state.replacements.clone();
            match to {
                Some(to) => all.insert(from, to),
                None => all.remove(&from),
            };
            proxy.call("SetReplacements", &(all,))
        }
        SpellingCommand::GetDictionary(language) => {
            if let Err(error) = get_dictionary(&language) {
                tracing::warn!("cannot get a dictionary for {language}: {error}");
            }
            Ok(())
        }
    }
}

pub fn follow_spelling(updates: Updates) -> Sender<SpellingCommand> {
    let (sender, receiver) = mpsc::channel();
    let watcher = updates.clone();
    spawn("spelling", move || {
        let connected = Connection::session().and_then(|connection| {
            let proxy = Proxy::new(&connection, BUS_NAME, OBJECT_PATH, BUS_NAME)?;
            Ok((connection, proxy))
        });
        let Ok((connection, proxy)) = connected else {
            return;
        };
        spawn("spelling-changes", move || {
            let Ok(proxy) = Proxy::new(&connection, BUS_NAME, OBJECT_PATH, BUS_NAME) else {
                return;
            };
            let Ok(changes) = proxy.receive_signal("Changed") else {
                return;
            };
            for _ in changes {
                if let Ok(state) = read(&proxy)
                    && !watcher.send(Update::Spelling(state))
                {
                    return;
                }
            }
        });
        let mut state = SpellingState::default();
        while let Ok(command) = receiver.recv() {
            if let Err(error) = apply(&proxy, command, &state) {
                tracing::warn!("cannot change spelling: {error}");
            }
            state = read(&proxy).unwrap_or(state);
            if !updates.send(Update::Spelling(state.clone())) {
                return;
            }
        }
    });
    sender
}

impl SettingsView {
    pub(crate) fn spelling_rows(&mut self) -> Widgets {
        let state = self.spelling.clone();
        let mut rows: Widgets = Vec::new();
        for (name, label) in SWITCHES {
            let label = L10N.tr(label);
            let on = state.options.get(name).copied().unwrap_or(true);
            let set = intent(&self.intents, move |on| {
                Intent::Spelling(SpellingCommand::Switch(name.to_owned(), on))
            });
            let toggle = boxed(Toggle::new(&label, on, set));
            rows.push(boxed(Row::new(
                Text::new(&label, Tone::Primary),
                Some(toggle),
            )));
        }
        for dictionary in &state.available {
            let on = state.active.contains(dictionary);
            let name = dictionary.clone();
            let set = intent(&self.intents, move |on| {
                Intent::Spelling(SpellingCommand::Dictionary(name.clone(), on))
            });
            let toggle = boxed(Toggle::new(dictionary, on, set));
            rows.push(boxed(Row::new(
                Text::new(dictionary, Tone::Primary),
                Some(toggle),
            )));
        }
        for name in &state.missing {
            let wanted = name.clone();
            let mut act = intent(&self.intents, move |()| {
                Intent::Spelling(SpellingCommand::GetDictionary(wanted.clone()))
            });
            let button = boxed(Button::new(
                &L10N.tr("spelling-get-dictionary"),
                move || act(()),
            ));
            rows.push(boxed(Row::new(
                Text::new(
                    &L10N.tr_args(
                        "spelling-missing-dictionary",
                        [("name", name.clone().into())],
                    ),
                    Tone::Primary,
                ),
                Some(button),
            )));
        }
        if state.learned.is_empty() {
            rows.push(muted_row(&L10N.tr("spelling-learned-none")));
        }
        for word in &state.learned {
            let forgotten = word.clone();
            let mut act = intent(&self.intents, move |()| {
                Intent::Spelling(SpellingCommand::Forget(forgotten.clone()))
            });
            let button = boxed(Button::new(&L10N.tr("remove"), move || act(())));
            rows.push(boxed(Row::new(
                Text::new(word, Tone::Primary),
                Some(button),
            )));
        }
        for (from, to) in &state.replacements {
            let removed = from.clone();
            let mut act = intent(&self.intents, move |()| {
                Intent::Spelling(SpellingCommand::SetReplacement(removed.clone(), None))
            });
            let button = boxed(Button::new(&L10N.tr("remove"), move || act(())));
            rows.push(boxed(Row::new(
                Text::new(&format!("{from} → {to}"), Tone::Primary),
                Some(button),
            )));
        }
        let entry = Rc::new(RefCell::new((String::new(), String::new())));
        let (from_entry, to_entry, add_entry) = (entry.clone(), entry.clone(), entry);
        let from = TextField::new(&L10N.tr("replace-from"), "", move |text| {
            text.clone_into(&mut from_entry.borrow_mut().0);
        })
        .without_text_checking();
        let to = TextField::new(&L10N.tr("replace-to"), "", move |text| {
            text.clone_into(&mut to_entry.borrow_mut().1);
        })
        .without_text_checking();
        let mut act = intent(&self.intents, move |()| {
            let (from, to) = add_entry.borrow().clone();
            Intent::Spelling(SpellingCommand::SetReplacement(from, Some(to)))
        });
        rows.push(boxed(from));
        rows.push(boxed(to));
        rows.push(boxed(Button::new(&L10N.tr("replace-add"), move || act(()))));
        rows
    }

    pub(crate) fn change_spelling(&mut self, command: SpellingCommand) {
        self.commands.push(Command::Spelling(command));
    }
}
