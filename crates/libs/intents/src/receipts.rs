use std::collections::HashMap;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
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
const PRIVATE_FOLDER: u32 = 0o700;
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

#[derive(Serialize, Deserialize)]
struct KeptStep {
    intent: String,
    arguments: Json,
    touched: Vec<(String, Vec<String>)>,
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
    pub(crate) fn is_old(&self) -> bool {
        self.made < now().saturating_sub(LASTS)
    }

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

fn remove_stored(path: &Path) {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            tracing::warn!("cannot remove {}: {error}", path.display());
        }
        _ => {}
    }
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
    let made = DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_FOLDER)
        .create(folder);
    let opened = made.and_then(|()| {
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

    fn steps(&self, token: &str) -> PathBuf {
        self.folder
            .join(&self.domain)
            .join(format!("{token}.steps"))
    }

    /// Adds a step to the undo of a job; without one, the last is left out.
    pub(crate) fn add_step(&self, token: &str, step: Option<&Undo>) -> io::Result<()> {
        let step = step.map(|undo| KeptStep {
            intent: undo.intent.clone(),
            arguments: undo.arguments.clone(),
            touched: undo.touched.clone(),
        });
        let mut line = serde_json::to_vec(&step).map_err(io::Error::other)?;
        line.push(b'\n');
        let path = self.steps(token);
        let folder = self.folder.join(&self.domain);
        DirBuilder::new()
            .recursive(true)
            .mode(PRIVATE_FOLDER)
            .create(&folder)?;
        let is_new = !path.exists();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(PRIVATE)
            .open(&path)?;
        let before = file.metadata()?.len();
        file.write_all(&line)
            .and_then(|()| file.sync_data())
            .and_then(|()| match is_new {
                true => bande_xdg::sync_folder(&folder),
                false => Ok(()),
            })
            .inspect_err(|_| {
                if let Err(error) = file.set_len(before) {
                    tracing::warn!("cannot take back a step that was not kept: {error}");
                }
            })
    }

    fn kept_steps(&self, token: &str) -> Vec<Option<KeptStep>> {
        let path = self.steps(token);
        let stored = match fs::read(&path) {
            Ok(stored) => stored,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(error) => {
                tracing::warn!("cannot read {}: {error}", path.display());
                return Vec::new();
            }
        };
        stored
            .split_inclusive(|byte| *byte == b'\n')
            .filter(|line| line.ends_with(b"\n"))
            .map_while(|line| {
                serde_json::from_slice(line)
                    .inspect_err(|error| tracing::warn!("a kept step cannot be read: {error}"))
                    .ok()
            })
            .collect()
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
        let mut beside = 0;
        let mut kept = Vec::new();
        let mut forgotten = Vec::new();
        for receipt in std::mem::take(&mut self.all).into_iter().rev() {
            beside += receipt.undo.as_ref().map_or(0, |undo| undo.beside);
            let begun = receipt.state == State::Begun;
            if begun || (!receipt.is_old() && kept.len() < MOST && beside <= MOST_BESIDE) {
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
        if receipt.undo.as_ref().is_some_and(|undo| undo.beside > 0) {
            remove_stored(&self.beside(&receipt.token));
        }
        remove_stored(&self.steps(&receipt.token));
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
        self.all.iter().find(|receipt| {
            receipt.token == token && receipt.state == State::Whole && !receipt.is_old()
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
        if let Some(replaced) = before.iter().find(|kept| kept.token == token) {
            if inline && replaced.undo.as_ref().is_some_and(|undo| undo.beside > 0) {
                remove_stored(&self.beside(&token));
            }
            remove_stored(&self.steps(&token));
        }
        Ok(())
    }

    /// Takes `left` as what the things must still look like.
    pub(crate) fn look_again(&mut self, token: &str, left: Vec<String>) -> io::Result<()> {
        let found = self.all.iter_mut().find(|receipt| receipt.token == token);
        let Some(undo) = found.and_then(|receipt| receipt.undo.as_mut()) else {
            return Ok(());
        };
        undo.left = left;
        self.save()
    }

    pub(crate) fn remove(&mut self, token: &str) -> io::Result<()> {
        let Some(at) = self.all.iter().position(|receipt| receipt.token == token) else {
            return Ok(());
        };
        let before = self.all.clone();
        let removed = self.all.remove(at);
        if let Err(error) = self.save() {
            self.all = before;
            return Err(error);
        }
        self.remove_beside(&removed);
        Ok(())
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
        let first = self.first_undo(receipt);
        if receipt.state == State::Whole {
            return first;
        }
        let mut steps: Vec<Undo> = first.into_iter().collect();
        for step in self.kept_steps(&receipt.token) {
            match step {
                Some(step) => steps.push(Undo {
                    intent: step.intent,
                    arguments: step.arguments,
                    touched: step.touched,
                }),
                None => {
                    steps.pop();
                }
            }
        }
        Undo::of_steps(&steps)
            .inspect_err(|refusal| tracing::warn!("an undo cannot be put together: {refusal}"))
            .ok()
            .flatten()
    }

    fn first_undo(&self, receipt: &Receipt) -> Option<Undo> {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn step(number: u32) -> Undo {
        Undo {
            intent: "books.restore".to_owned(),
            arguments: serde_json::json!({ "books": [number] }),
            touched: vec![(number.to_string(), Vec::new())],
        }
    }

    #[test]
    fn a_job_cut_short_is_undone_by_the_steps_that_were_kept() {
        let folder = tempfile::tempdir().unwrap();
        let mut receipts = Receipts::open(folder.path(), "books").unwrap();
        let undo = receipts.kept("job", step(1), Vec::new()).unwrap();
        receipts
            .keep(Receipt {
                token: "job".to_owned(),
                plan: String::new(),
                intent: "books.trash".to_owned(),
                summary: String::new(),
                references: Vec::new(),
                agent: None,
                made: now(),
                state: State::Begun,
                asked: Vec::new(),
                undo: Some(undo),
            })
            .unwrap();
        for number in [2, 3] {
            receipts.add_step("job", Some(&step(number))).unwrap();
        }
        receipts.add_step("job", None).unwrap();
        receipts.add_step("job", Some(&step(4))).unwrap();
        let steps = receipts.steps("job");
        drop(receipts);
        let mut file = OpenOptions::new().append(true).open(&steps).unwrap();
        file.write_all(b"{\"intent\": \"b\xc3").unwrap();

        let mut receipts = Receipts::open(folder.path(), "books").unwrap();
        let begun = receipts.all()[0].clone();
        let undo = receipts.undo(&begun).unwrap();
        assert_eq!(undo.arguments["books"], serde_json::json!([1, 2, 4]));
        assert_eq!(undo.touched.len(), 3);
        let whole = Receipt {
            state: State::Whole,
            undo: Some(receipts.kept("job", undo, Vec::new()).unwrap()),
            ..begun
        };
        receipts.keep(whole.clone()).unwrap();
        assert!(!steps.exists());
        assert_eq!(receipts.undo(&whole).unwrap().arguments["books"][2], 4);
    }
}
