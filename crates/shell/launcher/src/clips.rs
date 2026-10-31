use std::os::fd::BorrowedFd;
use std::sync::mpsc::{Sender, channel};

use unbound_inbox::Inbox;
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
    replies: Inbox<Reply>,
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
    pub fn spawn(session: zbus::blocking::Connection) -> std::io::Result<Self> {
        let (requests, wanted) = channel::<Request>();
        let replies = Inbox::new()?;
        let sender = replies.outbox();
        std::thread::Builder::new()
            .name("clipboard".to_owned())
            .spawn(move || {
                while let Ok(request) = wanted.recv() {
                    let answered = ClipboardProxyBlocking::new(&session)
                        .and_then(|proxy| answer(&proxy, request.clone()));
                    match answered {
                        Ok(Some(reply)) => {
                            if !sender.send(reply) {
                                return;
                            }
                        }
                        Ok(None) => {}
                        Err(error) => tracing::warn!("clipboard history {request:?}: {error}"),
                    }
                }
            })?;
        Ok(Self { requests, replies })
    }

    pub fn ask(&self, request: Request) {
        let _ = self.requests.send(request);
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.replies.fd()
    }

    pub fn take(&mut self) -> Vec<Reply> {
        self.replies.drain()
    }
}
