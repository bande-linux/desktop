use std::sync::mpsc::{self, Sender};

use bande_dict::Choices;
use bande_packages::{Client, Info, Mode};
use bande_toolkit::Spacing;
use bande_toolkit::ui::{Axis, Button, Stack, Toggle};

use crate::L10N;
use crate::follow::{Update, Updates, spawn};
use crate::pages::{Widgets, boxed};
use crate::view::{Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

const PACK_PACKAGES: &str = "bande-dict-";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Source {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LookUpState {
    sources: Vec<Source>,
    choices: Choices,
    available: Vec<(String, String)>,
    installing: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookUpCommand {
    Use(String, bool),
    First(String),
    Wikipedia(bool),
    Install(String),
}

fn installed_sources(choices: &Choices) -> Vec<Source> {
    let mut sources: Vec<Source> = bande_dict::installed()
        .into_iter()
        .map(|pack| Source {
            id: pack.id.clone(),
            name: pack.name.clone(),
        })
        .collect();
    sources.sort_by_key(|source| {
        choices
            .order
            .iter()
            .position(|id| *id == source.id)
            .unwrap_or(usize::MAX)
    });
    sources
}

fn available_packages() -> Vec<(String, String)> {
    let Ok(system) = bande_bus::system() else {
        return Vec::new();
    };
    Client::new(&system)
        .search_names(PACK_PACKAGES)
        .inspect_err(|error| tracing::warn!("cannot list the dictionaries: {error}"))
        .unwrap_or_default()
        .into_iter()
        .filter(|package| {
            package.info == Info::Available && package.name.starts_with(PACK_PACKAGES)
        })
        .map(|package| (package.id, package.summary))
        .collect()
}

fn install(id: &str) -> Result<(), bande_packages::Error> {
    let system = bande_bus::system()?;
    Client::new(&system)
        .install(&[id.to_owned()], Mode::Apply, &|_| {})
        .map(drop)
}

fn read() -> LookUpState {
    let choices = Choices::load();
    LookUpState {
        sources: installed_sources(&choices),
        available: available_packages(),
        choices,
        installing: None,
    }
}

fn apply(state: &LookUpState, command: LookUpCommand) {
    let mut choices = state.choices.clone();
    match command {
        LookUpCommand::Use(id, on) => {
            choices.off.retain(|off| *off != id);
            if !on {
                choices.off.push(id);
            }
        }
        LookUpCommand::First(id) => {
            choices.order = state
                .sources
                .iter()
                .map(|source| source.id.clone())
                .collect();
            choices.order.retain(|known| *known != id);
            choices.order.insert(0, id);
        }
        LookUpCommand::Wikipedia(on) => choices.wikipedia = on,
        LookUpCommand::Install(id) => {
            if let Err(error) = install(&id) {
                tracing::warn!("cannot install {id}: {error}");
            }
            return;
        }
    }
    if let Err(error) = choices.save() {
        tracing::warn!("cannot save the Look Up choices: {error}");
    }
}

pub fn follow_look_up(updates: Updates) -> Sender<LookUpCommand> {
    let (sender, receiver) = mpsc::channel();
    spawn("look-up", move || {
        let mut state = read();
        if !updates.send(Update::LookUp(state.clone())) {
            return;
        }
        while let Ok(command) = receiver.recv() {
            if let LookUpCommand::Install(id) = &command {
                state.installing = Some(id.clone());
                if !updates.send(Update::LookUp(state.clone())) {
                    return;
                }
            }
            apply(&state, command);
            state = read();
            if !updates.send(Update::LookUp(state.clone())) {
                return;
            }
        }
    });
    sender
}

impl SettingsView {
    pub(crate) fn look_up_rows(&mut self) -> Widgets {
        let state = self.look_up.clone();
        let mut rows: Widgets = Vec::new();
        for (index, source) in state.sources.iter().enumerate() {
            let on = !state.choices.off.contains(&source.id);
            let id = source.id.clone();
            let set = intent(&self.intents, move |on| {
                Intent::LookUp(LookUpCommand::Use(id.clone(), on))
            });
            let mut controls: Widgets = Vec::new();
            if index > 0 {
                let id = source.id.clone();
                let mut first = intent(&self.intents, move |()| {
                    Intent::LookUp(LookUpCommand::First(id.clone()))
                });
                controls.push(boxed(Button::new(&L10N.tr("look-up-first"), move || {
                    first(())
                })));
            }
            controls.push(boxed(Toggle::new(&source.name, on, set)));
            let row = Row::new(
                Text::new(&source.name, Tone::Primary),
                Some(boxed(Stack::new(
                    Axis::Horizontal,
                    Spacing::S as f32,
                    0.0,
                    controls,
                ))),
            );
            rows.push(boxed(row));
        }
        for (id, summary) in &state.available {
            let wanted = id.clone();
            let mut get = intent(&self.intents, move |()| {
                Intent::LookUp(LookUpCommand::Install(wanted.clone()))
            });
            let mut button = Button::new(&L10N.tr("look-up-install"), move || get(()));
            button.set_enabled(state.installing.is_none());
            rows.push(boxed(Row::new(
                Text::new(summary, Tone::Primary),
                Some(boxed(button)),
            )));
        }
        let set = intent(&self.intents, |on| {
            Intent::LookUp(LookUpCommand::Wikipedia(on))
        });
        let label = L10N.tr("look-up-wikipedia");
        rows.push(boxed(
            Row::new(
                Text::new(&label, Tone::Primary),
                Some(boxed(Toggle::new(&label, state.choices.wikipedia, set))),
            )
            .detail(Text::new(&L10N.tr("look-up-wikipedia-online"), Tone::Muted).caption()),
        ));
        rows
    }
}
