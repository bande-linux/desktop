use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};

use bande_l10n::tr;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zeroize::Zeroizing;

use bande_volumes::udisks::{ENCRYPTED, Objects, UDISKS};

const PROMPT: &str = "/usr/libexec/bande-secrets-prompt";
const LONGEST_ANSWER: u64 = 8192;
const FAILED: &str = "org.freedesktop.UDisks2.Error.Failed";
const KEY_REFUSED: [&str; 3] = [
    "Incorrect passphrase",
    "Operation not permitted",
    "No key available",
];

pub enum Unlocked {
    Opened(OwnedObjectPath),
    Cancelled,
    NoPrompt,
    Failed(zbus::Error),
}

fn single_line(text: &str) -> String {
    text.replace(['\r', '\n'], "\r").replace('\t', " ")
}

#[derive(Clone)]
pub struct Prompt(Arc<Mutex<ChildStdin>>);

impl Prompt {
    fn tell(&self, line: &str) {
        let mut input = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Err(error) = input
            .write_all(line.as_bytes())
            .and_then(|()| input.flush())
        {
            tracing::debug!("the passphrase prompt is gone: {error}");
        }
    }

    pub fn close(&self) {
        self.tell("done\n");
    }
}

fn unlock_with(
    system: &Connection,
    path: &OwnedObjectPath,
    passphrase: &str,
) -> zbus::Result<OwnedObjectPath> {
    let options: HashMap<&str, OwnedValue> = HashMap::new();
    let reply = system.call_method(
        Some(UDISKS),
        path,
        Some(ENCRYPTED),
        "Unlock",
        &(passphrase, options),
    )?;
    reply.body().deserialize()
}

fn wrong_passphrase(error: &zbus::Error) -> bool {
    matches!(error, zbus::Error::MethodError(name, Some(message), _)
        if name.as_str() == FAILED && KEY_REFUSED.iter().any(|reason| message.contains(reason)))
}

fn after_failure(system: &Connection, path: &OwnedObjectPath, error: zbus::Error) -> Unlocked {
    let objects = match Objects::load(system) {
        Ok(objects) => objects,
        Err(reload) => {
            tracing::debug!("cannot reread the disks: {reload}");
            return Unlocked::Failed(error);
        }
    };
    if !objects.has(path, ENCRYPTED) {
        return Unlocked::Cancelled;
    }
    objects
        .path(path, ENCRYPTED, "CleartextDevice")
        .and_then(|cleartext| OwnedObjectPath::try_from(cleartext).ok())
        .map_or(Unlocked::Failed(error), Unlocked::Opened)
}

pub fn ask_and_unlock(
    system: &Connection,
    path: &OwnedObjectPath,
    label: &str,
    shown: impl FnOnce(Prompt),
) -> Unlocked {
    if !Path::new(PROMPT).exists() {
        return Unlocked::NoPrompt;
    }
    let spawned = Command::new(PROMPT)
        .arg("--passphrase")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!("cannot start {PROMPT}: {error}");
            return Unlocked::NoPrompt;
        }
    };
    let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
        return Unlocked::NoPrompt;
    };
    let prompt = Prompt(Arc::new(Mutex::new(input)));
    let texts = [
        ("title", tr!("“{label}” is encrypted", label = label)),
        ("description", tr!("Enter its passphrase to open it.")),
        ("ok", tr!("Unlock")),
    ];
    let mut header: String = texts
        .iter()
        .map(|(key, value)| format!("{key}\t{}\n", single_line(value)))
        .collect();
    header.push('\n');
    prompt.tell(&header);
    shown(prompt.clone());
    let mut answers = BufReader::new(output.take(LONGEST_ANSWER));
    let outcome = loop {
        let mut line = Zeroizing::new(String::new());
        match answers.read_line(&mut line) {
            Ok(0) => break Unlocked::Cancelled,
            Ok(_) => {}
            Err(error) => {
                tracing::warn!("cannot read the passphrase: {error}");
                break Unlocked::Cancelled;
            }
        }
        let text = line.strip_suffix('\n').unwrap_or(&line);
        let Some(("allow", passphrase)) = text.split_once('\t') else {
            break Unlocked::Cancelled;
        };
        match unlock_with(system, path, passphrase) {
            Ok(cleartext) => break Unlocked::Opened(cleartext),
            Err(error) if wrong_passphrase(&error) => prompt.tell("wrong\n"),
            Err(error) => break after_failure(system, path, error),
        }
    };
    prompt.close();
    drop(prompt);
    if let Err(error) = child.wait() {
        tracing::debug!("cannot wait for the passphrase prompt: {error}");
    }
    outcome
}
