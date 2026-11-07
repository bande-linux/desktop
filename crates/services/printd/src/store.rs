use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

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
pub struct Pins(Arc<Mutex<HashMap<String, String>>>);

impl Pins {
    pub fn load() -> Self {
        let pins = state_folder()
            .and_then(|state| std::fs::read(state.join(PINS)).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self(Arc::new(Mutex::new(pins)))
    }

    pub fn check(&self, host: &str, print: &[u8]) -> bool {
        let hex: String = print.iter().map(|byte| format!("{byte:02x}")).collect();
        let mut pins = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match pins.get(host) {
            Some(known) => *known == hex,
            None => {
                pins.insert(host.to_owned(), hex);
                if let Some(state) = state_folder() {
                    let written = serde_json::to_string(&*pins)
                        .map_err(std::io::Error::other)
                        .and_then(|text| replace_file(&state.join(PINS), &text));
                    if let Err(error) = written {
                        tracing::warn!("cannot remember a printer's identity: {error}");
                    }
                }
                true
            }
        }
    }

    pub fn forget(&self, host: &str) {
        let mut pins = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if pins.remove(host).is_some()
            && let Some(state) = state_folder()
            && let Ok(text) = serde_json::to_string(&*pins)
            && let Err(error) = replace_file(&state.join(PINS), &text)
        {
            tracing::warn!("cannot forget a printer's identity: {error}");
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredJob {
    pub id: u32,
    pub title: String,
    pub printer: String,
    pub options: JobOptions,
}
