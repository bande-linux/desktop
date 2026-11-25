use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bande_l10n::tr;
use rustix::fs::{FlockOperation, flock};
use rustix::io::Errno;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};

use crate::{Dict, Effect, Refusal, Undo, plan};

const LASTS: u64 = 24 * 60 * 60;
const MOST: usize = 100;
const INLINE: usize = 64 * 1024;
const MOST_BESIDE: u64 = 1_000_000_000;
const LOCK_PATIENCE: Duration = Duration::from_secs(2);
const LOCK_RETRY: Duration = Duration::from_millis(20);
const PRIVATE: u32 = 0o600;
const PROBE: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State {
    Begun,
    Whole,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Receipt {
    pub(crate) token: String,
    pub(crate) plan: String,
    pub(crate) intent: String,
    pub(crate) summary: String,
    pub(crate) references: Vec<String>,
    pub(crate) agent: Option<String>,
    pub(crate) made: u64,
    pub(crate) state: State,
    pub(crate) asked: Vec<Effect>,
    pub(crate) undo: Option<KeptUndo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct KeptUndo {
    pub(crate) intent: String,
    arguments: Option<Json>,
    beside: u64,
    pub(crate) touched: Vec<(String, Vec<String>)>,
    pub(crate) left: Vec<String>,
}

impl Receipt {
    pub(crate) fn wire(&self, domain: &str) -> Result<Dict, Refusal> {
        let mut dict = Dict::new();
        plan::put(&mut dict, "receipt", format!("{domain}:{}", self.token))?;
        plan::put(&mut dict, "intent", self.intent.as_str())?;
        plan::put(&mut dict, "summary", self.summary.as_str())?;
        plan::put(&mut dict, "references", self.references.clone())?;
        plan::put(&mut dict, "made", i64::try_from(self.made).unwrap_or(0))?;
        plan::put(
            &mut dict,
            "agent",
            self.agent.as_deref().unwrap_or_default(),
        )?;
        Ok(dict)
    }
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

pub(crate) fn digest(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn unstored(error: &io::Error) -> Refusal {
    Refusal::unavailable(tr!(
        "Undo cannot be kept: {error}",
        error = error.to_string()
    ))
}

pub(crate) struct Receipts {
    folder: PathBuf,
    domain: String,
    all: Vec<Receipt>,
    pub(crate) interrupted: Vec<String>,
    _lock: File,
}

fn lock(folder: &Path, domain: &str) -> Result<File, Refusal> {
    let path = folder.join(format!("{domain}.lock"));
    let opened = fs::create_dir_all(folder).and_then(|()| {
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(PRIVATE)
            .open(&path)
    });
    let file =
        opened.map_err(|error| Refusal::unavailable(format!("{}: {error}", path.display())))?;
    let until = Instant::now() + LOCK_PATIENCE;
    loop {
        match flock(&file, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => return Ok(file),
            Err(Errno::WOULDBLOCK) if Instant::now() < until => std::thread::sleep(LOCK_RETRY),
            Err(Errno::INTR) => {}
            Err(_) => {
                return Err(Refusal::unavailable(tr!(
                    "Another program is using “{domain}”. Try again.",
                    domain = domain
                )));
            }
        }
    }
}

impl Receipts {
    pub(crate) fn open(folder: &Path, domain: &str) -> Result<Self, Refusal> {
        let held = lock(folder, domain)?;
        let mut receipts = Self {
            folder: folder.to_owned(),
            domain: domain.to_owned(),
            all: Vec::new(),
            interrupted: Vec::new(),
            _lock: held,
        };
        receipts.all = receipts.load();
        Ok(receipts)
    }

    fn path(&self) -> PathBuf {
        self.folder.join(format!("{}.json", self.domain))
    }

    fn beside(&self, token: &str) -> PathBuf {
        self.folder.join(&self.domain).join(format!("{token}.json"))
    }

    fn load(&self) -> Vec<Receipt> {
        let path = self.path();
        let stored = match fs::read(&path) {
            Ok(stored) => stored,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(error) => {
                tracing::warn!("cannot read {}: {error}", path.display());
                return Vec::new();
            }
        };
        let entries: Vec<Json> = serde_json::from_slice(&stored)
            .inspect_err(|error| tracing::warn!("cannot read {}: {error}", path.display()))
            .unwrap_or_default();
        entries
            .into_iter()
            .filter_map(|entry| {
                serde_json::from_value(entry)
                    .inspect_err(|error| tracing::warn!("a receipt cannot be read: {error}"))
                    .ok()
            })
            .collect()
    }

    fn save(&mut self) -> io::Result<()> {
        self.forget_old();
        let contents = serde_json::to_vec(&self.all).map_err(io::Error::other)?;
        bande_xdg::replace_private_file(&self.path(), contents)
    }

    fn forget_old(&mut self) {
        let oldest = now().saturating_sub(LASTS);
        let mut beside = 0;
        let mut kept = Vec::new();
        let mut forgotten = Vec::new();
        for receipt in std::mem::take(&mut self.all).into_iter().rev() {
            beside += receipt.undo.as_ref().map_or(0, |undo| undo.beside);
            let begun = receipt.state == State::Begun;
            if begun || (receipt.made >= oldest && kept.len() < MOST && beside <= MOST_BESIDE) {
                kept.push(receipt);
            } else {
                forgotten.push(receipt);
            }
        }
        kept.reverse();
        self.all = kept;
        for receipt in forgotten {
            self.remove_beside(&receipt);
        }
    }

    fn remove_beside(&self, receipt: &Receipt) {
        if receipt.undo.as_ref().is_some_and(|undo| undo.beside > 0)
            && let Err(error) = fs::remove_file(self.beside(&receipt.token))
        {
            tracing::warn!("cannot remove the undo of {}: {error}", receipt.intent);
        }
    }

    pub(crate) fn can_keep(&self) -> bool {
        let probe = self.folder.join(format!(".{}.probe", self.domain));
        let kept = fs::write(&probe, [0; PROBE]);
        let _ = fs::remove_file(&probe);
        kept.is_ok()
    }

    pub(crate) fn all(&self) -> &[Receipt] {
        &self.all
    }

    pub(crate) fn find(&self, token: &str) -> Option<&Receipt> {
        let oldest = now().saturating_sub(LASTS);
        self.all.iter().find(|receipt| {
            receipt.token == token && receipt.state == State::Whole && receipt.made >= oldest
        })
    }

    pub(crate) fn keep(&mut self, receipt: Receipt) -> io::Result<()> {
        let before = self.all.clone();
        let token = receipt.token.clone();
        let inline = receipt.undo.as_ref().is_none_or(|undo| undo.beside == 0);
        match self.all.iter_mut().find(|kept| kept.token == token) {
            Some(kept) => *kept = receipt,
            None => self.all.push(receipt),
        }
        if let Err(error) = self.save() {
            self.all = before;
            return Err(error);
        }
        if let Some(replaced) = before.iter().find(|kept| inline && kept.token == token) {
            self.remove_beside(replaced);
        }
        Ok(())
    }

    pub(crate) fn remove(&mut self, token: &str) -> io::Result<()> {
        let Some(at) = self.all.iter().position(|receipt| receipt.token == token) else {
            return Ok(());
        };
        let removed = self.all.remove(at);
        self.remove_beside(&removed);
        self.save()
    }

    pub(crate) fn kept(&self, token: &str, undo: Undo, left: Vec<String>) -> io::Result<KeptUndo> {
        let arguments = serde_json::to_vec(&undo.arguments).map_err(io::Error::other)?;
        let inline = arguments.len() <= INLINE;
        if !inline {
            bande_xdg::replace_private_file(&self.beside(token), &arguments)?;
        }
        Ok(KeptUndo {
            intent: undo.intent,
            beside: if inline { 0 } else { arguments.len() as u64 },
            arguments: inline.then_some(undo.arguments),
            touched: undo.touched,
            left,
        })
    }

    pub(crate) fn undo(&self, receipt: &Receipt) -> Option<Undo> {
        let kept = receipt.undo.as_ref()?;
        let arguments = match &kept.arguments {
            Some(arguments) => arguments.clone(),
            None => fs::read(self.beside(&receipt.token))
                .map_err(serde_json::Error::io)
                .and_then(|stored| serde_json::from_slice(&stored))
                .inspect_err(|error| tracing::warn!("cannot read an undo: {error}"))
                .ok()?,
        };
        Some(Undo {
            intent: kept.intent.clone(),
            arguments,
            touched: kept.touched.clone(),
        })
    }
}

pub(crate) type Open = HashMap<String, Receipts>;
