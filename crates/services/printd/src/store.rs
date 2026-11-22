use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bande_printers::JobOptions;
use bande_xdg::{replace_file, state_home};
use serde::{Deserialize, Serialize};

const FOLDER: &str = "bande";
const QUEUE: &str = "print-queue";
const PRINTERS: &str = "printers.json";
const PINS: &str = "printer-identities.json";

pub fn state_folder() -> Option<PathBuf> {
    state_home().map(|state| state.join(FOLDER))
}

pub fn queue_folder() -> Option<PathBuf> {
    state_folder().map(|state| state.join(QUEUE))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Known {
    pub name: String,
    pub model: String,
    pub location: String,
    pub plain: Option<String>,
    pub secure: Option<String>,
    pub by_address: bool,
    pub used: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Printers {
    pub default: Option<String>,
    pub known: HashMap<String, Known>,
}

impl Printers {
    pub fn load() -> Self {
        state_folder()
            .and_then(|state| std::fs::read(state.join(PRINTERS)).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(state) = state_folder() else {
            return;
        };
        let written = serde_json::to_string(self)
            .map_err(std::io::Error::other)
            .and_then(|text| replace_file(&state.join(PRINTERS), &text));
        if let Err(error) = written {
            tracing::warn!("cannot remember the printers: {error}");
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Pins {
    all: Arc<Mutex<HashMap<String, String>>>,
    printer: String,
}

impl Pins {
    pub fn load() -> Self {
        let pins = state_folder()
            .and_then(|state| std::fs::read(state.join(PINS)).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            all: Arc::new(Mutex::new(pins)),
            printer: String::new(),
        }
    }

    pub fn of(&self, printer: &str) -> Self {
        Self {
            all: Arc::clone(&self.all),
            printer: printer.to_owned(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, String>> {
        self.all.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn check(&self, print: &[u8]) -> bool {
        let hex: String = print.iter().map(|byte| format!("{byte:02x}")).collect();
        let mut pins = self.lock();
        match pins.get(&self.printer) {
            Some(known) => *known == hex,
            None => {
                pins.insert(self.printer.clone(), hex);
                save_pins(&pins, "cannot remember a printer's identity");
                true
            }
        }
    }

    pub fn forget(&self, printer: &str) {
        let mut pins = self.lock();
        if pins.remove(printer).is_some() {
            save_pins(&pins, "cannot forget a printer's identity");
        }
    }
}

fn save_pins(pins: &HashMap<String, String>, failure: &str) {
    let Some(state) = state_folder() else {
        return;
    };
    let written = serde_json::to_string(pins)
        .map_err(std::io::Error::other)
        .and_then(|text| replace_file(&state.join(PINS), &text));
    if let Err(error) = written {
        tracing::warn!("{failure}: {error}");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredJob {
    pub id: u32,
    pub title: String,
    pub printer: String,
    pub options: JobOptions,
}
