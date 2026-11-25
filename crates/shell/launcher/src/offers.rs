use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, SystemTime};

use bande_inbox::{Inbox, Outbox};
use bande_intents::phrases::{Offer, Phrases};
use bande_intents::{Client, Manifests, Refusal};
use bande_l10n::tr;
use bande_notify::Urgency;
use bande_notify::client::{Listener, Request, Signal, notify};
use zbus::blocking::Connection;
use zbus::blocking::connection::Builder;

const ANSWERED: Duration = Duration::from_secs(2);
const UNDO: &str = "undo";
const ICON: &str = "system-search-symbolic";

enum Asked {
    Typed(u64, String),
    Perform(Offer),
}

/// What can be done as one typed line, asked and done on a thread of its own.
pub struct Offers {
    asked: Sender<Asked>,
    offered: Inbox<(u64, Vec<Offer>)>,
}

struct Known {
    folder: PathBuf,
    stamp: Option<SystemTime>,
    manifests: Manifests,
    phrases: Phrases,
}

impl Known {
    fn read(folder: PathBuf) -> Self {
        let stamp = stamp(&folder);
        let manifests = Manifests::read(&folder);
        let phrases = Phrases::new(&manifests);
        Self {
            folder,
            stamp,
            manifests,
            phrases,
        }
    }

    fn follow(&mut self) {
        if stamp(&self.folder) != self.stamp {
            *self = Self::read(std::mem::take(&mut self.folder));
        }
    }
}

fn stamp(folder: &Path) -> Option<SystemTime> {
    std::fs::metadata(folder)
        .and_then(|metadata| metadata.modified())
        .ok()
}

impl Offers {
    /// Offers the phrases of the manifests in `folder`.
    pub fn spawn(folder: impl Into<PathBuf>) -> std::io::Result<Self> {
        let folder = folder.into();
        let (asked, questions) = channel();
        let offered = Inbox::new()?;
        let answers = offered.outbox();
        std::thread::Builder::new()
            .name("offers".to_owned())
            .spawn(move || serve(folder, &questions, &answers))?;
        Ok(Self { asked, offered })
    }

    pub fn ask(&self, generation: u64, typed: String) {
        let _ = self.asked.send(Asked::Typed(generation, typed));
    }

    /// Does what was offered, and tells the person with a notice that can undo it.
    pub fn perform(&self, offer: Offer) {
        let _ = self.asked.send(Asked::Perform(offer));
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.offered.fd()
    }

    pub fn take(&mut self) -> Option<(u64, Vec<Offer>)> {
        self.offered.drain().into_iter().next_back()
    }
}

fn serve(folder: PathBuf, questions: &Receiver<Asked>, answers: &Outbox<(u64, Vec<Offer>)>) {
    let mut known = Known::read(folder);
    while let Ok(first) = questions.recv() {
        let mut typed = None;
        for asked in std::iter::once(first).chain(questions.try_iter()) {
            match asked {
                Asked::Typed(generation, text) => typed = Some((generation, text)),
                Asked::Perform(offer) => spawn_perform(offer, &known.manifests),
            }
        }
        let Some((generation, text)) = typed else {
            continue;
        };
        known.follow();
        let offers = match text.trim().is_empty() || known.phrases.is_empty() {
            true => Vec::new(),
            false => answering_soon()
                .map(|connection| known.phrases.offers(&Client::new(&connection), &text))
                .inspect_err(|error| tracing::warn!("cannot ask what can be done: {error}"))
                .unwrap_or_default(),
        };
        if !answers.send((generation, offers)) {
            return;
        }
    }
}

fn answering_soon() -> zbus::Result<Connection> {
    Builder::session()?.method_timeout(ANSWERED).build()
}

fn spawn_perform(offer: Offer, manifests: &Manifests) {
    let manifests = manifests.clone();
    let spawned = std::thread::Builder::new()
        .name("offer".to_owned())
        .spawn(move || {
            let shown = bande_bus::session()
                .and_then(|connection| perform(&connection, &manifests, &offer));
            if let Err(error) = shown {
                tracing::warn!("cannot tell what was done: {error}");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("cannot do what was chosen: {error}");
    }
}

fn perform(connection: &Connection, manifests: &Manifests, offer: &Offer) -> zbus::Result<()> {
    let client = Client::new(connection);
    let (summary, receipt) = match offer.perform(&client) {
        Ok(done) => done,
        Err(refusal) => return refused(connection, &offer.title, &refusal),
    };
    let Some(receipt) = receipt else {
        return post(connection, &summary, "", &[]).map(|_| ());
    };
    let listener = Listener::new(connection)?;
    let shown = post(connection, &summary, "", &[(UNDO, &tr!("Undo"))])?;
    let mut undone = Ok(());
    listener.listen(|signal| match signal {
        Signal::ActionInvoked(id, key) if id == shown && key == UNDO => {
            if let Err(refusal) = client.undo(manifests, &receipt) {
                undone = refused(connection, &summary, &refusal);
            }
            false
        }
        Signal::Closed(id) => id != shown,
        _ => true,
    });
    undone
}

fn refused(connection: &Connection, title: &str, refusal: &Refusal) -> zbus::Result<()> {
    post(connection, title, &refusal.message, &[]).map(|_| ())
}

fn post(
    connection: &Connection,
    summary: &str,
    body: &str,
    actions: &[(&str, &str)],
) -> zbus::Result<u32> {
    notify(
        connection,
        &Request {
            app_name: &tr!("Search"),
            replaces_id: 0,
            app_icon: ICON,
            summary,
            body,
            actions,
            urgency: Urgency::Normal,
            sound: None,
            desktop_entry: None,
        },
    )
}
