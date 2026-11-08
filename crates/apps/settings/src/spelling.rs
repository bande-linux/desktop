use std::collections::BTreeMap;
use std::sync::mpsc::{self, Sender};

use bande_l10n::{text, tr};
use bande_toolkit::ui::{Button, Label, TextField, Toggle, Widget};
use zbus::blocking::Proxy;

use crate::follow::{Command, Update, Updates, spawn};
use crate::pages::{Widgets, boxed, muted_row};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Columns, Row, SIDEBAR_SYMBOL, Text, Tone};

const BUS_NAME: &str = "org.bande.Spell";
const ADD_SYMBOL: &str = "list-add-symbolic";
const REMOVE_SYMBOL: &str = "list-remove-symbolic";
const ADD_ANCHOR: &str = "replacement-add";
const FROM_ANCHOR: &str = "replacement-from";
const TO_ANCHOR: &str = "replacement-to";
const ROW_ANCHOR: &str = "replacement:";
const OBJECT_PATH: &str = "/org/bande/Spell";
const SWITCHES: [(&str, bande_l10n::Text); 6] = [
    ("spelling", text!("Check spelling")),
    ("autocorrect", text!("Correct spelling automatically")),
    ("capitalise", text!("Capitalise words automatically")),
    ("double-space", text!("Add period with double-space")),
    ("smart-quotes", text!("Use smart quotes and dashes")),
    ("links", text!("Show web and email addresses as links")),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacing {
    original: Option<String>,
    from: String,
    to: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementField {
    From,
    To,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SpellingCommand {
    Read,
    Switch(String, bool),
    Dictionary(String, bool),
    Forget(String),
    GetDictionary(String),
    Replace {
        old: Option<String>,
        new: Option<(String, String)>,
    },
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
        SpellingCommand::Replace { old, new } => {
            let mut all = state.replacements.clone();
            if let Some(old) = old {
                all.remove(&old);
            }
            if let Some((from, to)) = new {
                all.insert(from, to);
            }
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
        let connected = bande_bus::session().and_then(|connection| {
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
            let label = label.tr();
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
            let button = boxed(Button::new(&tr!("Get Dictionary"), move || act(())));
            rows.push(boxed(Row::new(
                Text::new(
                    &tr!(
                        "The {name} dictionary matches your region but isn't installed.",
                        name = name.clone()
                    ),
                    Tone::Primary,
                ),
                Some(button),
            )));
        }
        if state.learned.is_empty() {
            rows.push(muted_row(&tr!("No learned words yet.")));
        }
        for word in &state.learned {
            let forgotten = word.clone();
            let mut act = intent(&self.intents, move |()| {
                Intent::Spelling(SpellingCommand::Forget(forgotten.clone()))
            });
            let button = boxed(Button::new(&tr!("Remove"), move || act(())));
            rows.push(boxed(Row::new(
                Text::new(word, Tone::Primary),
                Some(button),
            )));
        }
        rows
    }

    pub(crate) fn change_spelling(&mut self, command: SpellingCommand) {
        self.commands.push(Command::Spelling(command));
    }

    pub(crate) fn text_replacement_rows(&mut self) -> Widgets {
        let replacements = self.spelling.replacements.clone();
        let editing = self.replacing.clone();
        let mut add = intent(&self.intents, |()| Intent::EditReplacement(None));
        let title = tr!("Add Text Replacement");
        let ink = self.palette().ink2;
        let button = match self.symbol(ADD_SYMBOL, SIDEBAR_SYMBOL, ink) {
            Some(symbol) => Button::symbol(symbol, &title, move || add(())),
            None => Button::new(&title, move || add(())),
        };
        self.anchors
            .push((Anchor::Item(ADD_ANCHOR.to_owned()), button.id()));
        let mut rows: Widgets = vec![boxed(
            Columns::new(
                &tr!("Replace, With"),
                boxed(Label::new(&tr!("Replace")).secondary()),
                boxed(Label::new(&tr!("With")).secondary()),
                Some(boxed(button)),
            )
            .heading(),
        )];
        for (from, to) in &replacements {
            match &editing {
                Some(edit) if edit.original.as_deref() == Some(from.as_str()) => {
                    rows.push(self.replacement_editor(edit));
                }
                _ => rows.push(self.replacement_row(from, to)),
            }
        }
        match &editing {
            Some(edit) if edit.original.is_none() => rows.push(self.replacement_editor(edit)),
            _ if replacements.is_empty() => {
                rows.push(muted_row(&tr!("No text replacements yet.")));
            }
            _ => {}
        }
        rows
    }

    fn replacement_row(&mut self, from: &str, to: &str) -> Box<dyn Widget> {
        let original = from.to_owned();
        let mut edit = intent(&self.intents, move |()| {
            Intent::EditReplacement(Some(original.clone()))
        });
        let row = Columns::new(
            &tr!(
                "{from}, replaced with {to}",
                from = from.to_owned(),
                to = to.to_owned()
            ),
            boxed(Label::new(from)),
            boxed(Label::new(to)),
            Some(self.remove_replacement_button(from)),
        )
        .on_activate(move || edit(()));
        self.anchors
            .push((Anchor::Item(format!("{ROW_ANCHOR}{from}")), row.id()));
        boxed(row)
    }

    fn replacement_editor(&mut self, edit: &Replacing) -> Box<dyn Widget> {
        let mut from_changed = intent(&self.intents, |text: String| {
            Intent::ReplacementText(ReplacementField::From, text)
        });
        let mut to_changed = intent(&self.intents, |text: String| {
            Intent::ReplacementText(ReplacementField::To, text)
        });
        let from = TextField::new(&tr!("Replace"), &edit.from, move |text: &str| {
            from_changed(text.to_owned())
        })
        .without_text_checking();
        let to = TextField::new(&tr!("With"), &edit.to, move |text: &str| {
            to_changed(text.to_owned())
        })
        .without_text_checking();
        self.anchors
            .push((Anchor::Item(FROM_ANCHOR.to_owned()), from.id()));
        self.anchors
            .push((Anchor::Item(TO_ANCHOR.to_owned()), to.id()));
        let remove = edit
            .original
            .as_deref()
            .map(|original| self.remove_replacement_button(original));
        boxed(Columns::new(&tr!("Replace, With"), boxed(from), boxed(to), remove).fields())
    }

    fn remove_replacement_button(&mut self, from: &str) -> Box<dyn Widget> {
        let removed = from.to_owned();
        let mut remove = intent(&self.intents, move |()| {
            Intent::Spelling(SpellingCommand::Replace {
                old: Some(removed.clone()),
                new: None,
            })
        });
        let title = tr!("Remove Text Replacement");
        let ink = self.palette().ink2;
        boxed(match self.symbol(REMOVE_SYMBOL, SIDEBAR_SYMBOL, ink) {
            Some(symbol) => Button::symbol(symbol, &title, move || remove(())),
            None => Button::new(&title, move || remove(())),
        })
    }

    pub(crate) fn replacement_focused(&self) -> bool {
        [FROM_ANCHOR, TO_ANCHOR]
            .iter()
            .any(|anchor| self.anchor_focused(&Anchor::Item((*anchor).to_owned())))
    }

    pub(crate) fn edit_replacement(&mut self, original: Option<String>) {
        self.commit_replacement();
        let to = original
            .as_ref()
            .and_then(|from| self.spelling.replacements.get(from))
            .cloned()
            .unwrap_or_default();
        self.replacing = Some(Replacing {
            from: original.clone().unwrap_or_default(),
            to,
            original,
        });
        self.focus_next = Some(Anchor::Item(FROM_ANCHOR.to_owned()));
    }

    pub(crate) fn type_replacement(&mut self, field: ReplacementField, text: String) {
        if let Some(edit) = &mut self.replacing {
            match field {
                ReplacementField::From => edit.from = text,
                ReplacementField::To => edit.to = text,
            }
        }
    }

    pub(crate) fn commit_replacement(&mut self) {
        let Some(edit) = self.replacing.take() else {
            return;
        };
        self.stale = true;
        let (from, to) = (edit.from.trim(), edit.to.trim());
        if from.is_empty() || to.is_empty() {
            return;
        }
        let unchanged = edit.original.as_deref() == Some(from)
            && self.spelling.replacements.get(from).map(String::as_str) == Some(to);
        if unchanged {
            return;
        }
        self.focus_next = Some(Anchor::Item(format!("{ROW_ANCHOR}{from}")));
        let new = Some((from.to_owned(), to.to_owned()));
        self.change_spelling(SpellingCommand::Replace {
            old: edit.original,
            new,
        });
    }

    pub(crate) fn cancel_replacement(&mut self) {
        if let Some(edit) = self.replacing.take() {
            self.stale = true;
            self.focus_next = Some(Anchor::Item(match edit.original {
                Some(original) => format!("{ROW_ANCHOR}{original}"),
                None => ADD_ANCHOR.to_owned(),
            }));
        }
    }
}
