use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use zbus::message::Header;

use crate::history::{Contents, History, summary, text_of};

pub const BUS_NAME: &str = "org.bande.Clipboard";
pub const OBJECT_PATH: &str = "/org/bande/Clipboard";
pub const INTERFACE: &str = "org.bande.Clipboard";
const SCREEN_SHARING: &str = "bande-screen-sharing.service";
const TEXT_MIMES: [&str; 3] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"];

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

async fn from_screen_sharing(
    header: &Header<'_>,
    connection: &zbus::Connection,
) -> zbus::fdo::Result<()> {
    let denied =
        || zbus::fdo::Error::AccessDenied("only Screen Sharing reads and sets the text".into());
    let sender = header.sender().ok_or_else(denied)?.to_owned();
    let bus = zbus::blocking::Connection::from(connection.clone());
    let allowed = blocking::unblock(move || {
        bande_bus::is_main_process_of(&bus, sender.into(), SCREEN_SHARING)
    })
    .await?;
    match allowed {
        true => Ok(()),
        false => Err(denied()),
    }
}

#[zbus::interface(name = "org.bande.Clipboard")]
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

    async fn text(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<String> {
        from_screen_sharing(&header, connection).await?;
        Ok(lock(&self.history)
            .entries()
            .first()
            .and_then(|entry| text_of(&entry.contents))
            .unwrap_or_default())
    }

    async fn copy_text(
        &mut self,
        text: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        from_screen_sharing(&header, connection).await?;
        let contents: Contents = TEXT_MIMES
            .iter()
            .map(|mime| ((*mime).to_owned(), text.as_bytes().to_vec()))
            .collect();
        if !lock(&self.history).record(contents.clone(), now()) {
            return Err(zbus::fdo::Error::InvalidArgs(
                "the text cannot be copied".into(),
            ));
        }
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
