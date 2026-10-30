use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender, channel};

use unbound_preview::Preview;
use unbound_toolkit::Image;

use crate::model::Clip;

type Item = (u64, u64, String, Vec<String>);

#[zbus::proxy(
    interface = "org.unbound.Clipboard",
    default_service = "org.unbound.Clipboard",
    default_path = "/org/unbound/Clipboard"
)]
trait Clipboard {
    fn items(&self) -> zbus::Result<Vec<Item>>;
    fn content(&self, id: u64, mime: &str) -> zbus::Result<Vec<u8>>;
    fn paste(&self, id: u64) -> zbus::Result<()>;
    fn remove(&self, id: u64) -> zbus::Result<()>;
    fn clear(&self) -> zbus::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    List,
    Picture(u64, String),
    Paste(u64),
    Remove(u64),
    Clear,
}

pub enum Reply {
    Listed(Vec<Clip>),
    Picture(u64, Preview<Image>),
}

pub struct Clips {
    requests: Sender<Request>,
    replies: Receiver<Reply>,
    pub wake: UnixStream,
}

fn decode(bytes: &[u8]) -> Option<Image> {
    let decoded = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = decoded.dimensions();
    Image::from_rgba(width, height, decoded.into_raw())
}

fn answer(proxy: &ClipboardProxyBlocking<'_>, request: Request) -> zbus::Result<Option<Reply>> {
    Ok(match request {
        Request::List => Some(Reply::Listed(
            proxy
                .items()?
                .into_iter()
                .map(|(id, copied, text, mimes)| Clip {
                    id,
                    copied,
                    text,
                    mimes,
                })
                .collect(),
        )),
        Request::Picture(id, mime) => {
            let bytes = proxy.content(id, &mime)?;
            Some(Reply::Picture(
                id,
                decode(&bytes).map_or(
                    Preview::Other {
                        size: bytes.len() as u64,
                    },
                    Preview::Image,
                ),
            ))
        }
        Request::Paste(id) => {
            proxy.paste(id)?;
            None
        }
        Request::Remove(id) => {
            proxy.remove(id)?;
            None
        }
        Request::Clear => {
            proxy.clear()?;
            None
        }
    })
}

impl Clips {
    pub fn spawn() -> std::io::Result<Self> {
        let (requests, wanted) = channel::<Request>();
        let (sender, replies) = channel();
        let (wake, signal) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        signal.set_nonblocking(true)?;
        std::thread::Builder::new()
            .name("clipboard".to_owned())
            .spawn(move || {
                let connection = match zbus::blocking::Connection::session() {
                    Ok(connection) => connection,
                    Err(error) => {
                        tracing::warn!("cannot reach the clipboard history: {error}");
                        return;
                    }
                };
                while let Ok(request) = wanted.recv() {
                    let answered = ClipboardProxyBlocking::new(&connection)
                        .and_then(|proxy| answer(&proxy, request.clone()));
                    match answered {
                        Ok(Some(reply)) => {
                            if sender.send(reply).is_err() {
                                return;
                            }
                            let _ = (&signal).write(&[1]);
                        }
                        Ok(None) => {}
                        Err(error) => tracing::warn!("clipboard history {request:?}: {error}"),
                    }
                }
            })?;
        Ok(Self {
            requests,
            replies,
            wake,
        })
    }

    pub fn ask(&self, request: Request) {
        let _ = self.requests.send(request);
    }

    pub fn take(&self) -> Vec<Reply> {
        let mut drained = [0; 64];
        while matches!((&self.wake).read(&mut drained), Ok(read) if read > 0) {}
        self.replies.try_iter().collect()
    }
}
