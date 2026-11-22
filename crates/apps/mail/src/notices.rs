use std::collections::HashMap;
use std::os::fd::BorrowedFd;

use bande_inbox::{Inbox, Outbox};
use bande_l10n::tr;
use bande_mail::store::MessageId;
use bande_notify::Urgency;
use bande_notify::client::{Request, Signal, listen, notify};
use bande_taskbar_model::bus::show_badge;
use zbus::blocking::Connection;

const OPEN: &str = "default";
const NEW_MAIL_SOUND: &str = "message-new-email";

enum Notice {
    Shown(u32, Option<MessageId>),
    Signal(Signal),
}

pub struct Clicked {
    pub message: Option<MessageId>,
    pub token: Option<String>,
}

#[derive(Clone)]
pub struct Announcer {
    app_id: &'static str,
    session: Option<Connection>,
    outbox: Outbox<Notice>,
}

pub struct Notices {
    announcer: Announcer,
    inbox: Inbox<Notice>,
    shown: HashMap<u32, Option<MessageId>>,
    tokens: HashMap<u32, String>,
    badge: u32,
}

fn in_background(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(work)
    {
        tracing::warn!("cannot start the {name} thread: {error}");
    }
}

impl Announcer {
    pub fn announce(&self, summary: String, body: String, sound: bool, message: Option<MessageId>) {
        let Some(session) = self.session.clone() else {
            return tracing::debug!("no session bus for a notification");
        };
        let app_id = self.app_id;
        let outbox = self.outbox.clone();
        in_background("notify", move || {
            let shown = notify(
                &session,
                &Request {
                    app_name: &tr!("Mail"),
                    replaces_id: 0,
                    app_icon: app_id,
                    summary: &summary,
                    body: &body,
                    actions: &[(OPEN, "")],
                    urgency: Urgency::Normal,
                    sound: sound.then_some(NEW_MAIL_SOUND),
                    desktop_entry: Some(app_id),
                },
            );
            match shown {
                Ok(id) => {
                    outbox.send(Notice::Shown(id, message));
                }
                Err(error) => tracing::debug!("cannot show a notification: {error}"),
            }
        });
    }
}

impl Notices {
    pub fn start(app_id: &'static str, session: Option<&Connection>) -> std::io::Result<Self> {
        let inbox = Inbox::new()?;
        if let Some(session) = session.cloned() {
            let outbox = inbox.outbox();
            in_background("notices", move || {
                if let Err(error) = listen(&session, |signal| outbox.send(Notice::Signal(signal))) {
                    tracing::warn!("cannot follow new mail notifications: {error}");
                }
            });
        }
        Ok(Self {
            announcer: Announcer {
                app_id,
                session: session.cloned(),
                outbox: inbox.outbox(),
            },
            inbox,
            shown: HashMap::new(),
            tokens: HashMap::new(),
            badge: 0,
        })
    }

    pub fn announcer(&self) -> &Announcer {
        &self.announcer
    }

    pub fn set_badge(&mut self, count: u32) {
        if count == self.badge {
            return;
        }
        self.badge = count;
        let Some(session) = &self.announcer.session else {
            return;
        };
        if let Err(error) = show_badge(session, self.announcer.app_id, count, false) {
            tracing::warn!("cannot show the unread count in the Taskbar: {error}");
        }
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.inbox.fd()
    }

    pub fn clicked(&mut self) -> Vec<Clicked> {
        let mut clicked = Vec::new();
        for notice in self.inbox.drain() {
            match notice {
                Notice::Shown(id, message) => {
                    self.shown.insert(id, message);
                }
                Notice::Signal(Signal::ActivationToken(id, token)) => {
                    if self.shown.contains_key(&id) {
                        self.tokens.insert(id, token);
                    }
                }
                Notice::Signal(Signal::ActionInvoked(id, key)) => {
                    if key == OPEN
                        && let Some(message) = self.shown.remove(&id)
                    {
                        clicked.push(Clicked {
                            message,
                            token: self.tokens.remove(&id),
                        });
                    }
                }
                Notice::Signal(Signal::Closed(id)) => {
                    self.shown.remove(&id);
                    self.tokens.remove(&id);
                }
            }
        }
        clicked
    }
}
