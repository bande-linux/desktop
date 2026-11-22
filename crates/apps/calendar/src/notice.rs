use std::collections::HashMap;
use std::os::fd::BorrowedFd;

use bande_datetime::DateFormats;
use bande_inbox::{Inbox, Outbox};
use bande_l10n::tr;
use bande_notify::client::{self, Signal};
use jiff::tz::TimeZone;
use zbus::blocking::Connection;
use zbus::zvariant::Value;

use crate::alerts::Due;

const APP_ID: &str = "org.bande.Calendar";
const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
const NO_TIMEOUT: i32 = -1;
pub const OPEN: &str = "default";
pub const SNOOZE: &str = "snooze";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    Alert(Due),
    SignIn(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Shown(u32, Subject),
    Invoked(u32, String),
    Closed(u32),
}

pub struct Notices {
    inbox: Inbox<Notice>,
}

fn listen(outbox: &Outbox<Notice>) -> zbus::Result<()> {
    client::listen(&Connection::session()?, |signal| match signal {
        Signal::ActionInvoked(id, key) => outbox.send(Notice::Invoked(id, key)),
        Signal::Closed(id) => outbox.send(Notice::Closed(id)),
        Signal::ActivationToken(..) => true,
    })
}

fn show(category: &str, title: &str, body: &str, actions: &[&str]) -> zbus::Result<u32> {
    let hints: HashMap<&str, Value> = HashMap::from([
        ("desktop-entry", Value::from(APP_ID)),
        ("category", Value::from(category)),
    ]);
    Connection::session()?
        .call_method(
            Some(NOTIFICATIONS),
            NOTIFICATIONS_PATH,
            Some(NOTIFICATIONS),
            "Notify",
            &(
                tr!("Calendar"),
                0u32,
                APP_ID,
                title,
                body,
                actions,
                hints,
                NO_TIMEOUT,
            ),
        )?
        .body()
        .deserialize::<u32>()
}

fn in_background(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(work)
    {
        tracing::warn!("cannot start the {name} thread: {error}");
    }
}

impl Notices {
    pub fn start() -> std::io::Result<Self> {
        let inbox = Inbox::new()?;
        let outbox = inbox.outbox();
        std::thread::Builder::new()
            .name("notices".to_owned())
            .spawn(move || {
                if let Err(error) = listen(&outbox) {
                    tracing::warn!("cannot follow alert notifications: {error}");
                }
            })?;
        Ok(Self { inbox })
    }

    pub fn notify(&self, due: Due, zone: &TimeZone, formats: &DateFormats) {
        let starts = due.starts.to_zoned(zone.clone());
        let when = match due.all_day {
            true => formats.date(&starts, false),
            false => formats.clock(&starts),
        };
        let body = match due.location.is_empty() {
            true => when,
            false => format!("{when}\n{}", due.location),
        };
        let outbox = self.inbox.outbox();
        in_background("alert", move || {
            let snooze = tr!("Snooze");
            match show(
                "x-calendar.alert",
                &due.title,
                &body,
                &[OPEN, "", SNOOZE, snooze.as_str()],
            ) {
                Ok(id) => {
                    tracing::debug!("alert for {}", due.title);
                    outbox.send(Notice::Shown(id, Subject::Alert(due)));
                }
                Err(error) => tracing::warn!("cannot show an alert: {error}"),
            }
        });
    }

    pub fn sign_in_failed(&self, account: String, name: &str) {
        let title = tr!("Can’t sign in to “{name}”", name = name);
        let body = tr!("The server refused the password. Click to enter it again.");
        let outbox = self.inbox.outbox();
        in_background("sign-in", move || {
            match show("x-calendar.sign-in", &title, &body, &[OPEN, ""]) {
                Ok(id) => {
                    outbox.send(Notice::Shown(id, Subject::SignIn(account)));
                }
                Err(error) => tracing::warn!("cannot tell about the refused password: {error}"),
            }
        });
    }

    pub fn withdraw(&self, id: u32) {
        in_background("withdraw", move || {
            if let Err(error) =
                Connection::session().and_then(|session| client::close(&session, id))
            {
                tracing::debug!("cannot withdraw a notification: {error}");
            }
        });
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.inbox.fd()
    }

    pub fn take(&mut self) -> Vec<Notice> {
        self.inbox.drain()
    }
}
