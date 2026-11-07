use std::fs;
use std::path::PathBuf;
use std::process::Command as Process;
use std::sync::mpsc::{self, Sender};

use bande_toolkit::ui::Toggle;

use crate::L10N;
use crate::follow::{Command, Update, Updates, spawn};
use crate::pages::{Widgets, boxed, muted_row};
use crate::view::{Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

const GRANTED_DIRECTORY: &str = "/var/lib/bande/consent";
const ASKED_FILE: &str = "bande/consent-asked";
const HELPER: &str = "/usr/libexec/bande-consent";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardAccess {
    pub line: String,
    pub allowed: bool,
}

impl KeyboardAccess {
    fn parts(&self) -> Option<(&str, &str, &str)> {
        let mut words = self.line.split(' ');
        Some((words.next()?, words.next()?, words.next()?))
    }

    fn label(&self) -> Option<String> {
        let (access, _, name) = self.parts()?;
        let program = name.rsplit('/').next().unwrap_or(name);
        let key = match access {
            "read-keys" => "keyboard-access-reads",
            "type-keys" => "keyboard-access-types",
            _ => return None,
        };
        Some(L10N.tr_args(key, [("program", program.into())]))
    }
}

fn granted_file() -> PathBuf {
    PathBuf::from(GRANTED_DIRECTORY).join(rustix::process::getuid().as_raw().to_string())
}

fn lines_of(path: Option<PathBuf>) -> Vec<String> {
    path.and_then(|path| fs::read_to_string(path).ok())
        .map(|text| text.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

fn read_all() -> Vec<KeyboardAccess> {
    let granted = lines_of(Some(granted_file()));
    let asked = lines_of(bande_xdg::state_home().map(|state| state.join(ASKED_FILE)));
    let mut all: Vec<KeyboardAccess> = granted
        .iter()
        .map(|line| KeyboardAccess {
            line: line.clone(),
            allowed: true,
        })
        .collect();
    for line in asked {
        if !granted.contains(&line) {
            all.push(KeyboardAccess {
                line,
                allowed: false,
            });
        }
    }
    all.sort_by(|a, b| a.line.cmp(&b.line));
    all
}

fn change(access: &KeyboardAccess, on: bool) -> std::io::Result<()> {
    let Some((capability, kind, name)) = access.parts() else {
        return Ok(());
    };
    Process::new("pkexec")
        .args([
            HELPER,
            capability,
            kind,
            name,
            if on { "on" } else { "off" },
        ])
        .status()
        .map(drop)
}

pub fn follow_keyboard_access(updates: Updates) -> Sender<(String, bool)> {
    let (sender, receiver) = mpsc::channel::<(String, bool)>();
    spawn("keyboard-access", move || {
        if !updates.send(Update::KeyboardAccess(read_all())) {
            return;
        }
        while let Ok((line, on)) = receiver.recv() {
            let access = KeyboardAccess { line, allowed: !on };
            if let Err(error) = change(&access, on) {
                tracing::warn!("cannot change keyboard access: {error}");
            }
            if !updates.send(Update::KeyboardAccess(read_all())) {
                return;
            }
        }
    });
    sender
}

impl SettingsView {
    pub(crate) fn keyboard_access_rows(&mut self) -> Widgets {
        if self.keyboard_access.is_empty() {
            return vec![muted_row(&L10N.tr("keyboard-access-none"))];
        }
        let mut rows: Widgets = Vec::new();
        for access in self.keyboard_access.clone() {
            let Some(label) = access.label() else {
                continue;
            };
            let line = access.line.clone();
            let set = intent(&self.intents, move |on| {
                Intent::KeyboardAccess(line.clone(), on)
            });
            let toggle = boxed(Toggle::new(&label, access.allowed, set));
            rows.push(boxed(Row::new(
                Text::new(&label, Tone::Primary),
                Some(toggle),
            )));
        }
        rows
    }

    pub(crate) fn change_keyboard_access(&mut self, line: String, on: bool) {
        self.commands.push(Command::KeyboardAccess(line, on));
    }
}
