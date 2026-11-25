use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};
use std::thread;

use zbus::blocking::Connection;

use crate::service::{IndexProxyBlocking, Offer};

struct Snapshot {
    domain: &'static str,
    fingerprint: String,
    items: Vec<Offer>,
}

fn take_snapshot(snapshot: Snapshot) -> zbus::Result<()> {
    let session = Connection::session()?;
    let index = IndexProxyBlocking::new(&session)?;
    if index.cursor(snapshot.domain)? != snapshot.fingerprint {
        index.forget_all(snapshot.domain)?;
        index.offer(snapshot.domain, snapshot.items, &snapshot.fingerprint)?;
    }
    Ok(())
}

fn sender() -> Option<&'static Sender<Snapshot>> {
    static OFFERS: OnceLock<Option<Sender<Snapshot>>> = OnceLock::new();
    OFFERS
        .get_or_init(|| {
            let (sender, received) = channel::<Snapshot>();
            thread::Builder::new()
                .name("search-offers".to_owned())
                .spawn(move || {
                    while let Ok(mut snapshot) = received.recv() {
                        while let Ok(newer) = received.try_recv() {
                            snapshot = newer;
                        }
                        if let Err(error) = take_snapshot(snapshot) {
                            tracing::debug!("search has not taken the items: {error}");
                        }
                    }
                })
                .ok()
                .map(|_| sender)
        })
        .as_ref()
}

/// Hands `items` to the search index on a thread of its own, unless the index already has
/// the snapshot named `fingerprint`; a newer snapshot replaces one still waiting.
pub fn offer_in_background(domain: &'static str, fingerprint: String, items: Vec<Offer>) {
    if let Some(sender) = sender() {
        let _ = sender.send(Snapshot {
            domain,
            fingerprint,
            items,
        });
    }
}
