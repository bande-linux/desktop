use std::env;
use std::ffi::{OsStr, OsString};

use xkbcommon::xkb::compose::{COMPILE_NO_FLAGS, FeedResult, STATE_NO_FLAGS, State, Status, Table};
use xkbcommon::xkb::{self, Keysym, keysyms};

const UTF8_LOCALE: &str = "C.UTF-8";
const COMPOSE_KEY_MARK: &str = "·";

#[derive(Default)]
pub(crate) struct Compose {
    state: Option<State>,
    pending: Vec<Keysym>,
}

pub(crate) struct Step {
    pub(crate) text: String,
    pub(crate) preedit: String,
    pub(crate) deliver_key: bool,
}

impl Step {
    fn key_with(text: String) -> Self {
        Self {
            text,
            preedit: String::new(),
            deliver_key: true,
        }
    }
}

fn is_dead(keysym: Keysym) -> bool {
    (keysyms::KEY_dead_grave..=keysyms::KEY_dead_longsolidusoverlay).contains(&keysym.raw())
}

fn starts_sequence(keysym: Keysym) -> bool {
    is_dead(keysym) || keysym.raw() == keysyms::KEY_Multi_key
}

fn is_utf8(locale: &OsStr) -> bool {
    let locale = locale.to_string_lossy().to_ascii_lowercase();
    locale.contains("utf-8") || locale.contains("utf8")
}

fn load_table() -> Option<Table> {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let session = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|name| env::var_os(name).filter(|value| !value.is_empty()))
        .filter(|locale| is_utf8(locale));
    let table = session
        .into_iter()
        .chain([OsString::from(UTF8_LOCALE)])
        .find_map(|locale| Table::new_from_locale(&context, &locale, COMPILE_NO_FLAGS).ok());
    if table.is_none() {
        tracing::warn!("no Compose table for this locale: dead keys type nothing");
    }
    table
}

fn composed_text(state: &State) -> Option<String> {
    state.utf8().or_else(|| {
        state
            .keysym()
            .and_then(|keysym| char::from_u32(xkb::keysym_to_utf32(keysym)))
            .filter(|character| *character != '\0')
            .map(String::from)
    })
}

fn sequence_text(table: &Table, first: Keysym, second: Keysym) -> Option<String> {
    let mut state = State::new(table, STATE_NO_FLAGS);
    state.feed(first);
    state.feed(second);
    (state.status() == Status::Composed)
        .then(|| composed_text(&state))
        .flatten()
}

fn spacing_accent(table: &Table, dead: Keysym) -> String {
    sequence_text(table, dead, dead)
        .or_else(|| sequence_text(table, dead, Keysym::new(keysyms::KEY_space)))
        .unwrap_or_default()
}

impl Compose {
    pub(crate) fn is_composing(&self) -> bool {
        !self.pending.is_empty()
    }

    pub(crate) fn cancel(&mut self) {
        self.pending.clear();
        if let Some(state) = &mut self.state {
            state.reset();
        }
    }

    pub(crate) fn feed(&mut self, keysym: Keysym) -> Step {
        if self.pending.is_empty() && !starts_sequence(keysym) {
            return Step::key_with(String::new());
        }
        if self.state.is_none() {
            self.state = load_table().map(|table| State::new(&table, STATE_NO_FLAGS));
        }
        let Some(state) = &mut self.state else {
            return Step::key_with(String::new());
        };
        if state.feed(keysym) == FeedResult::Ignored {
            let mut step = Step::key_with(String::new());
            step.preedit = self.shown();
            return step;
        }
        match state.status() {
            Status::Nothing => Step::key_with(String::new()),
            Status::Composing => {
                self.pending.push(keysym);
                Step {
                    text: String::new(),
                    preedit: self.shown(),
                    deliver_key: false,
                }
            }
            Status::Composed => {
                let text = self.composed(keysym);
                self.cancel();
                Step {
                    text,
                    preedit: String::new(),
                    deliver_key: false,
                }
            }
            Status::Cancelled => {
                let accents = self.pending_accents();
                self.cancel();
                let mut step = self.feed(keysym);
                step.text.insert_str(0, &accents);
                step
            }
        }
    }

    fn composed(&self, last: Keysym) -> String {
        let Some(state) = &self.state else {
            return String::new();
        };
        let table = state.compose_table();
        match self.pending[..] {
            [dead] if is_dead(dead) && last.raw() == keysyms::KEY_space => {
                spacing_accent(&table, dead)
            }
            _ => composed_text(state).unwrap_or_default(),
        }
    }

    fn pending_accents(&self) -> String {
        let Some(state) = &self.state else {
            return String::new();
        };
        let table = state.compose_table();
        self.pending
            .iter()
            .filter(|keysym| is_dead(**keysym))
            .map(|dead| spacing_accent(&table, *dead))
            .collect()
    }

    fn shown(&self) -> String {
        let Some(state) = &self.state else {
            return String::new();
        };
        let table = state.compose_table();
        self.pending
            .iter()
            .map(|keysym| match keysym.raw() {
                keysyms::KEY_Multi_key => COMPOSE_KEY_MARK.to_owned(),
                _ if is_dead(*keysym) => spacing_accent(&table, *keysym),
                _ => char::from_u32(xkb::keysym_to_utf32(*keysym))
                    .map(String::from)
                    .unwrap_or_default(),
            })
            .collect()
    }
}
