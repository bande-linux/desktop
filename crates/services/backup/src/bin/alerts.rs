use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::LazyLock;

use bande_backup::client::{ALERT_FILE, problem};
use bande_l10n::Localizer;
use bande_notify::Urgency;
use bande_notify::client::{self, Request};
use zbus::blocking::{Connection, MessageIterator};
use zbus::{MatchRule, message};

static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-backup")));

const OPEN: &str = "open-settings";
const SETTINGS: &str = "org.bande.Settings";
const NOTIFICATIONS: &str = "org.freedesktop.Notifications";

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

fn told_file() -> Option<PathBuf> {
    bande_xdg::state_home().map(|state| state.join("bande/backup-told"))
}

fn signals(session: &Connection) -> zbus::Result<MessageIterator> {
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .sender(NOTIFICATIONS)?
        .interface(NOTIFICATIONS)?
        .build();
    MessageIterator::for_match_rule(rule, session, None)
}

fn wait_for_answer(signals: MessageIterator, id: u32) {
    for signal in signals.flatten() {
        let header = signal.header();
        let body = signal.body();
        match header.member().map(|member| member.as_str()) {
            Some("ActionInvoked") => {
                if let Ok((shown, key)) = body.deserialize::<(u32, String)>()
                    && shown == id
                    && key == OPEN
                {
                    let command = ["bande-settings".to_owned(), "backup".to_owned()];
                    if let Err(error) = bande_xdg::spawn_detached(SETTINGS, &command, None, None) {
                        tracing::warn!("cannot open Settings: {error}");
                    }
                    return;
                }
            }
            Some("NotificationClosed")
                if body
                    .deserialize::<(u32, u32)>()
                    .is_ok_and(|(closed, _)| closed == id) =>
            {
                return;
            }
            _ => {}
        }
    }
}

fn run() -> zbus::Result<()> {
    let Ok(alert) = fs::read_to_string(ALERT_FILE) else {
        return Ok(());
    };
    let mut lines = alert.lines();
    let kind = lines.next().unwrap_or_default();
    let since = lines.next().unwrap_or_default();
    let disk = lines.next().unwrap_or_default().to_owned();
    let Some((title, body)) = texts(kind) else {
        return Ok(());
    };
    let told = format!("{kind} {since}");
    let told_path = told_file();
    if told_path
        .as_ref()
        .and_then(|path| fs::read_to_string(path).ok())
        .is_some_and(|known| known == told)
    {
        return Ok(());
    }
    let session = Connection::session()?;
    let signals = signals(&session)?;
    let open = L10N.tr("open-settings");
    let id = client::notify(
        &session,
        &Request {
            app_name: &L10N.tr("backup"),
            replaces_id: 0,
            app_icon: "drive-harddisk",
            summary: &L10N.tr(title),
            body: &L10N.tr_args(body, [("disk", disk.into())]),
            actions: &[(OPEN, &open)],
            urgency: Urgency::Normal,
            sound: None,
            desktop_entry: None,
        },
    )?;
    if let Some(path) = told_path {
        let remembered = path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(&path, told));
        if let Err(error) = remembered {
            tracing::warn!("cannot remember the backup alert: {error}");
        }
    }
    wait_for_answer(signals, id);
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
