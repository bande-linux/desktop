use std::process::ExitCode;
use std::sync::LazyLock;
use std::sync::mpsc::{self, Sender};
use std::thread;

use bande_backup::client::{Status, follow_status, problem};
use bande_l10n::Localizer;
use bande_notify::Urgency;
use bande_notify::client::{self, Request};
use zbus::blocking::{Connection, MessageIterator};
use zbus::{MatchRule, message};

const ENGLISH: &str = include_str!("../../locales/en-US/bande-backup.ftl");
static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

const OPEN: &str = "open-settings";
const SETTINGS: &str = "org.bande.Settings";

fn texts(kind: &str) -> Option<(&'static str, &'static str)> {
    match kind {
        problem::OVERDUE => Some(("overdue-title", "overdue-body")),
        problem::DISK_FULL => Some(("disk-full-title", "disk-full-body")),
        problem::DAMAGED => Some(("damaged-title", "damaged-body")),
        problem::CANNOT_OPEN => Some(("cannot-open-title", "cannot-open-body")),
        problem::FAILING => Some(("failing-title", "failing-body")),
        _ => None,
    }
}

fn tell(session: &Connection, status: &Status) -> Option<u32> {
    let (title, body) = texts(&status.problem)?;
    let open = L10N.tr("open-settings");
    let posted = client::notify(
        session,
        &Request {
            app_name: &L10N.tr("backup"),
            replaces_id: 0,
            app_icon: "drive-harddisk",
            summary: &L10N.tr(title),
            body: &L10N.tr_args(body, [("disk", status.disk_name.clone().into())]),
            actions: &[(OPEN, &open)],
            urgency: Urgency::Normal,
            sound: None,
        },
    );
    match posted {
        Ok(id) => Some(id),
        Err(error) => {
            tracing::warn!("cannot show the backup alert: {error}");
            None
        }
    }
}

enum Event {
    Status(Status),
    Action(u32),
}

fn follow_actions(session: &Connection, events: Sender<Event>) -> zbus::Result<()> {
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .interface("org.freedesktop.Notifications")?
        .member("ActionInvoked")?
        .build();
    let signals = MessageIterator::for_match_rule(rule, session, None)?;
    thread::spawn(move || {
        for signal in signals.flatten() {
            if let Ok((id, key)) = signal.body().deserialize::<(u32, String)>()
                && key == OPEN
                && events.send(Event::Action(id)).is_err()
            {
                return;
            }
        }
    });
    Ok(())
}

fn run() -> zbus::Result<()> {
    let system = Connection::system()?;
    let session = Connection::session()?;
    let (events, received) = mpsc::channel();
    follow_actions(&session, events.clone())?;
    follow_status(&system, move |status| match status {
        Ok(status) => events.send(Event::Status(status)).is_ok(),
        Err(error) => {
            tracing::debug!("cannot read the backup status: {error}");
            true
        }
    })?;
    let mut told = String::new();
    let mut shown = None;
    for event in received {
        match event {
            Event::Status(status) => {
                if status.problem != told {
                    told.clone_from(&status.problem);
                    shown = tell(&session, &status);
                }
            }
            Event::Action(id) if shown == Some(id) => {
                let command = ["bande-settings".to_owned(), "backup".to_owned()];
                if let Err(error) = bande_xdg::spawn_detached(SETTINGS, &command, None, None) {
                    tracing::warn!("cannot open Settings: {error}");
                }
            }
            Event::Action(_) => {}
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    bande_log::init("bande-backup-alerts");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
