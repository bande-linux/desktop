use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::history::{Contents, History, summary};

pub const BUS_NAME: &str = "org.unbound.Clipboard";
pub const OBJECT_PATH: &str = "/org/unbound/Clipboard";

pub type Shared = Arc<Mutex<History>>;
pub type Item = (u64, u64, String, Vec<String>);

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

pub fn lock(history: &Shared) -> MutexGuard<'_, History> {
    history
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub struct ClipboardHistory {
    pub history: Shared,
    pub paste: Sender<Contents>,
    pub wake: UnixStream,
}

#[zbus::interface(name = "org.unbound.Clipboard")]
impl ClipboardHistory {
    fn items(&self) -> Vec<Item> {
        let mut history = lock(&self.history);
        history.trim(now());
        history
            .entries()
            .iter()
            .map(|entry| {
                (
                    entry.id,
                    entry.copied,
                    summary(&entry.contents),
                    entry
                        .contents
                        .iter()
                        .map(|(mime, _)| mime.clone())
                        .collect(),
                )
            })
            .collect()
    }

    fn content(&self, id: u64, mime: &str) -> zbus::fdo::Result<Vec<u8>> {
        lock(&self.history)
            .get(id)
            .and_then(|entry| {
                entry
                    .contents
                    .iter()
                    .find(|(offered, _)| offered == mime)
                    .map(|(_, bytes)| bytes.clone())
            })
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no {mime} in item {id}")))
    }

    fn paste(&mut self, id: u64) -> zbus::fdo::Result<()> {
        let contents = lock(&self.history)
            .promote(id, now())
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no item {id}")))?;
        self.paste
            .send(contents)
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        self.wake
            .write_all(&[1])
            .map_err(|error| zbus::fdo::Error::IOError(error.to_string()))
    }

    fn remove(&self, id: u64) {
        lock(&self.history).remove(id);
    }

    fn clear(&self) {
        lock(&self.history).clear();
    }
}
